//! Typed forward CSR and inbound CSC views opened once per engine.

use alloc::vec::Vec;

use oxgraph_csc::{CscNodeId, CscSnapshotGraph};
use oxgraph_csr::{CsrNodeId, CsrSnapshotGraph};
use oxgraph_graph::{EdgeTargetGraph, OutgoingGraph, OutgoingNeighborsGraph, TopologyCounts};
use oxgraph_snapshot::Snapshot;

use crate::{
    artifact::{
        SNAPSHOT_KIND_PG_INBOUND_OFFSETS_U32, SNAPSHOT_KIND_PG_INBOUND_TARGETS_U32, read_metadata,
    },
    error::{BuildError, PostgresGraphError},
    overlay::OverlayState,
};

/// Section version the Postgres inbound CSC sections are written under, matching
/// the CSR section version the layout export uses.
const INBOUND_SECTION_VERSION: u32 = oxgraph_csr::SNAPSHOT_CSR_SECTION_VERSION;

/// Outgoing adjacency — foundation CSR topology sections only.
#[derive(Clone, Copy, Debug)]
pub struct ForwardCsr<'view>(pub(crate) CsrSnapshotGraph<'view, u32, u32>);

/// Incoming adjacency — Postgres inbound CSC sections only.
#[derive(Clone, Copy, Debug)]
pub struct InboundCsc<'view>(pub(crate) CscSnapshotGraph<'view>);

/// Both topology views borrowing the same snapshot backing.
#[derive(Clone, Copy, Debug, yoke::Yokeable)]
#[yoke(prove_covariant)]
pub struct GraphTopology<'view> {
    /// Forward CSR over outgoing edges.
    pub forward: ForwardCsr<'view>,
    /// Inbound CSC over predecessor lists.
    pub inbound: InboundCsc<'view>,
}

/// Hot topology slice views for BFS (assembled once per query).
#[derive(Clone, Copy, Debug)]
#[expect(
    clippy::redundant_pub_crate,
    reason = "shared with traverse session module"
)]
pub(crate) struct TopologyHot<'view> {
    /// Forward CSR for outgoing expansion.
    pub forward: ForwardCsr<'view>,
    /// Inbound CSC for incoming expansion.
    pub inbound: InboundCsc<'view>,
}

impl<'view> TopologyHot<'view> {
    /// Builds hot views from opened engine topology.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub(crate) const fn from_topology(topology: &GraphTopology<'view>) -> Self {
        Self {
            forward: topology.forward,
            inbound: topology.inbound,
        }
    }
}

/// Engine-local node-unique adjacency derived from the parallel base topology.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[expect(
    clippy::redundant_pub_crate,
    reason = "shared with engine and traversal modules"
)]
pub(crate) struct UniqueAdjacency {
    /// Outgoing row offsets into [`Self::outgoing_targets`].
    outgoing_offsets: Vec<usize>,
    /// Sorted, deduplicated outgoing target ids.
    outgoing_targets: Vec<u32>,
    /// Incoming row offsets into [`Self::incoming_sources`].
    incoming_offsets: Vec<usize>,
    /// Sorted, deduplicated incoming predecessor ids.
    incoming_sources: Vec<u32>,
}

impl UniqueAdjacency {
    /// Builds node-unique outgoing and incoming adjacency from base topology views.
    ///
    /// # Performance
    ///
    /// This method is `O(n + m log d)` for `n` nodes, `m` parallel edge slots, and maximum
    /// per-node degree `d`; it allocates `O(n + u)` memory for `u` unique adjacency slots.
    #[must_use]
    pub(crate) fn from_topology(forward: &ForwardCsr<'_>, inbound: &InboundCsc<'_>) -> Self {
        let node_count = forward.node_count();
        let (outgoing_offsets, outgoing_targets) =
            Self::build_unique_rows(node_count, |node| forward.successors(node));
        let (incoming_offsets, incoming_sources) =
            Self::build_unique_rows(node_count, |node| inbound.predecessors(node));
        Self {
            outgoing_offsets,
            outgoing_targets,
            incoming_offsets,
            incoming_sources,
        }
    }

    /// Builds sorted, deduplicated rows from a parallel adjacency row iterator.
    fn build_unique_rows<I>(
        node_count: usize,
        mut neighbors: impl FnMut(u32) -> I,
    ) -> (Vec<usize>, Vec<u32>)
    where
        I: Iterator<Item = u32>,
    {
        let mut offsets = Vec::with_capacity(node_count.saturating_add(1));
        let mut targets = Vec::new();
        let mut scratch = Vec::new();
        offsets.push(0);
        let Ok(node_bound) = u32::try_from(node_count) else {
            return (offsets, targets);
        };
        for node_id in 0..node_bound {
            scratch.clear();
            scratch.extend(neighbors(node_id));
            scratch.sort_unstable();
            scratch.dedup();
            targets.extend_from_slice(&scratch);
            offsets.push(targets.len());
        }
        (offsets, targets)
    }

    /// Returns the row slice for `node`, or an empty slice when out of bounds.
    fn row<'a>(offsets: &[usize], targets: &'a [u32], node: u32) -> &'a [u32] {
        let index = node as usize;
        let Some((&start, &end)) = offsets.get(index).zip(offsets.get(index.saturating_add(1)))
        else {
            return &[];
        };
        &targets[start..end]
    }

    /// Returns sorted, node-unique outgoing targets for `source`.
    ///
    /// # Performance
    ///
    /// This method is `O(1)` to borrow the row.
    #[must_use]
    pub(crate) fn outgoing(&self, source: u32) -> &[u32] {
        Self::row(&self.outgoing_offsets, &self.outgoing_targets, source)
    }

    /// Returns sorted, node-unique incoming predecessors for `target`.
    ///
    /// # Performance
    ///
    /// This method is `O(1)` to borrow the row.
    #[must_use]
    pub(crate) fn incoming(&self, target: u32) -> &[u32] {
        Self::row(&self.incoming_offsets, &self.incoming_sources, target)
    }
}

impl GraphTopology<'_> {
    /// Returns whether `node` is visible for traversal under `direction`.
    #[must_use]
    pub(crate) fn node_visible(
        &self,
        node: u32,
        direction: crate::traverse::TraversalDirection,
        overlay: &OverlayState,
    ) -> bool {
        match direction {
            crate::traverse::TraversalDirection::Out => self.forward.node_visible(node, overlay),
            crate::traverse::TraversalDirection::In => self.inbound.node_visible(node, overlay),
        }
    }
}

impl<'view> GraphTopology<'view> {
    /// Opens both layouts from validated snapshot bytes.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresGraphError`] when metadata, sections, or cross-layout counts disagree.
    ///
    /// # Performance
    ///
    /// This function is `O(s + n + m)`.
    pub fn open(snapshot: &Snapshot<'view>) -> Result<Self, PostgresGraphError> {
        let metadata = read_metadata(snapshot)?;
        if !metadata.has_reverse_index() {
            return Err(PostgresGraphError::Build(BuildError::MissingReverseIndex));
        }
        let forward = ForwardCsr(CsrSnapshotGraph::from_snapshot(snapshot)?);
        let inbound = InboundCsc(CscSnapshotGraph::from_snapshot_with_kinds(
            snapshot,
            SNAPSHOT_KIND_PG_INBOUND_OFFSETS_U32,
            SNAPSHOT_KIND_PG_INBOUND_TARGETS_U32,
            INBOUND_SECTION_VERSION,
        )?);
        if forward.0.element_count() != inbound.0.node_count() {
            return Err(PostgresGraphError::Build(
                BuildError::TopologyNodeCountMismatch,
            ));
        }
        if forward.0.relation_count() != inbound.0.relation_count() {
            return Err(PostgresGraphError::Build(
                BuildError::TopologyEdgeCountMismatch,
            ));
        }
        if u32::try_from(forward.0.element_count()).ok() != Some(metadata.node_count.get()) {
            return Err(PostgresGraphError::Build(
                BuildError::MetadataNodeCountMismatch,
            ));
        }
        if u32::try_from(forward.0.relation_count()).ok() != Some(metadata.edge_count.get()) {
            return Err(PostgresGraphError::Build(
                BuildError::MetadataEdgeCountMismatch,
            ));
        }
        Ok(Self { forward, inbound })
    }
}

impl ForwardCsr<'_> {
    /// Returns the node count in this forward view.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub fn node_count(&self) -> usize {
        self.0.element_count()
    }

    /// Returns successor node ids for `source`.
    ///
    /// # Performance
    ///
    /// This method is `O(1)` to create and `O(k)` to yield `k` successors.
    #[must_use]
    pub fn successors(&self, source: u32) -> impl ExactSizeIterator<Item = u32> + '_ {
        self.0
            .outgoing_neighbors(CsrNodeId::new(source))
            .map(CsrNodeId::get)
    }

    /// Returns whether `node` is visible for traversal seeds and results.
    #[must_use]
    pub(crate) fn node_visible(&self, node: u32, overlay: &OverlayState) -> bool {
        (node as usize) < self.node_count()
            && (!overlay.has_node_tombstones() || overlay.node_visible(node))
    }

    /// Walks outgoing target node ids for `source` via the CSR target slice.
    pub(crate) fn for_each_out_target(
        &self,
        source: u32,
        mut visit: impl FnMut(u32) -> bool,
    ) -> bool {
        self.0
            .for_each_out_target(CsrNodeId::new(source), |id| visit(id.get()))
    }

    /// Walks parallel outgoing `(target, edge_id)` slots for `source`.
    ///
    /// Stops early when `visit` returns `true`.
    ///
    /// # Performance
    ///
    /// This method is `O(k)` for `k` outgoing edges.
    pub(crate) fn for_each_out_edge(
        &self,
        source: u32,
        mut visit: impl FnMut(u32, u32) -> bool,
    ) -> bool {
        let graph = &self.0;
        for edge in OutgoingGraph::outgoing_edges(graph, CsrNodeId::new(source)) {
            let target = EdgeTargetGraph::target(graph, edge).get();
            if visit(target, edge.get()) {
                return true;
            }
        }
        false
    }
}

impl InboundCsc<'_> {
    /// Returns the node count in this inbound view.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub fn node_count(&self) -> usize {
        self.0.node_count()
    }

    /// Returns predecessor node ids for `target`.
    ///
    /// # Performance
    ///
    /// This method is `O(1)` to create and `O(k)` to yield `k` predecessors.
    #[must_use]
    pub fn predecessors(&self, target: u32) -> impl ExactSizeIterator<Item = u32> + '_ {
        self.0
            .predecessors(CscNodeId::new(target))
            .map(CscNodeId::get)
    }

    /// Returns whether `node` is visible for traversal seeds and results.
    #[must_use]
    pub(crate) fn node_visible(&self, node: u32, overlay: &OverlayState) -> bool {
        (node as usize) < self.node_count()
            && (!overlay.has_node_tombstones() || overlay.node_visible(node))
    }

    /// Walks predecessor node ids for `target` via the CSC source slice.
    ///
    /// Stops early when `visit` returns `true`.
    ///
    /// # Performance
    ///
    /// This method is `O(k)` for `k` predecessors with no iterator adapters.
    pub(crate) fn for_each_in_source(
        &self,
        target: u32,
        mut visit: impl FnMut(u32) -> bool,
    ) -> bool {
        self.0
            .for_each_predecessor(CscNodeId::new(target), |pred| visit(pred.get()))
    }
}
