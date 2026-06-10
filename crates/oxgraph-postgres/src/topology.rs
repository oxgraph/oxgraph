//! Typed forward CSR and inbound CSC views opened once per engine.

use alloc::vec::Vec;

use oxgraph_csc::{CscNodeId, CscSnapshotGraph};
use oxgraph_csr::{CsrNodeId, CsrSnapshotGraph};
use oxgraph_graph::{
    ContainsElement, DenseElementIndex, EdgeTargetGraph, ElementPredecessors, ElementSuccessors,
    OutgoingGraph, OutgoingNeighborsGraph, TopologyBase, TopologyCounts,
};
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
}

/// Borrowed neighbor-resolution flags shared by both overlay topology views.
///
/// These are resolved once per query by the traverse profile and select the
/// node-unique vs. parallel base walk, whether overlay adjacency is chained in,
/// and whether node tombstones must be filtered during expansion.
#[derive(Clone, Copy, Debug)]
#[expect(
    clippy::redundant_pub_crate,
    reason = "shared with the traverse view-construction path"
)]
pub(crate) struct OverlayViewFlags {
    /// Yield node-unique (sorted, deduplicated) base neighbors instead of the
    /// parallel CSR/CSC walk.
    pub use_unique: bool,
    /// Chain overlay adjacency after the base neighbors.
    pub merge_overlay: bool,
    /// Filter tombstoned nodes from both yielded and expanded elements.
    pub check_nodes: bool,
}

/// Outgoing overlay topology view binding the algo forward bounded BFS.
///
/// Wraps the base forward CSR plus the live overlay and resolves neighbors per
/// [`OverlayViewFlags`]: node-unique deduplicated rows (the `UniqueAdjacency`
/// semantics) or the parallel CSR walk with per-edge tombstone filtering, with
/// overlay targets chained after the base row. Tombstoned nodes are neither
/// yielded nor expanded.
#[derive(Clone, Copy, Debug)]
#[expect(
    clippy::redundant_pub_crate,
    reason = "constructed by the traverse module from engine-owned state"
)]
pub(crate) struct OverlayOutView<'view> {
    /// Base forward CSR adjacency.
    forward: ForwardCsr<'view>,
    /// Live overlay edges and tombstones.
    overlay: &'view OverlayState,
    /// Engine-cached node-unique rows (borrowed for the unique path).
    unique: &'view UniqueAdjacency,
    /// Resolved neighbor-resolution flags.
    flags: OverlayViewFlags,
    /// Canonical exclusive node bound.
    node_count: usize,
}

/// Inbound overlay topology view binding the algo reverse bounded BFS.
///
/// Wraps the base inbound CSC plus the forward CSR (needed for the edge
/// tombstone slow path) and the live overlay. Predecessors are resolved per
/// [`OverlayViewFlags`]: node-unique deduplicated rows or the parallel CSC walk,
/// with overlay sources chained after the base row. When edge tombstones are
/// active a predecessor `p` of `current` is visible only when the forward edge
/// `p -> current` is not tombstoned.
#[derive(Clone, Copy, Debug)]
#[expect(
    clippy::redundant_pub_crate,
    reason = "constructed by the traverse module from engine-owned state"
)]
pub(crate) struct OverlayInView<'view> {
    /// Base inbound CSC adjacency.
    inbound: InboundCsc<'view>,
    /// Base forward CSR (anchors the edge-tombstone forward-edge check).
    forward: ForwardCsr<'view>,
    /// Live overlay edges and tombstones.
    overlay: &'view OverlayState,
    /// Engine-cached node-unique rows (borrowed for the unique path).
    unique: &'view UniqueAdjacency,
    /// Resolved neighbor-resolution flags.
    flags: OverlayViewFlags,
    /// Canonical exclusive node bound.
    node_count: usize,
}

impl<'view> OverlayOutView<'view> {
    /// Builds the outgoing overlay view from engine-owned topology and overlay.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub(crate) const fn new(
        topology: &GraphTopology<'view>,
        overlay: &'view OverlayState,
        unique: &'view UniqueAdjacency,
        flags: OverlayViewFlags,
        node_count: usize,
    ) -> Self {
        Self {
            forward: topology.forward,
            overlay,
            unique,
            flags,
            node_count,
        }
    }
}

impl<'view> OverlayInView<'view> {
    /// Builds the inbound overlay view from engine-owned topology and overlay.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub(crate) const fn new(
        topology: &GraphTopology<'view>,
        overlay: &'view OverlayState,
        unique: &'view UniqueAdjacency,
        flags: OverlayViewFlags,
        node_count: usize,
    ) -> Self {
        Self {
            inbound: topology.inbound,
            forward: topology.forward,
            overlay,
            unique,
            flags,
            node_count,
        }
    }
}

/// Returns whether a forward edge `source -> target` is visible under edge
/// tombstones (the documented inbound parallel slow path).
fn has_visible_forward_edge(
    forward: &CsrSnapshotGraph<'_, u32, u32>,
    overlay: &OverlayState,
    source: u32,
    target: u32,
) -> bool {
    let target_id = CsrNodeId::new(target);
    for edge in OutgoingGraph::outgoing_edges(forward, CsrNodeId::new(source)) {
        if EdgeTargetGraph::target(forward, edge) == target_id && overlay.edge_visible(edge.get()) {
            return true;
        }
    }
    false
}

/// Base outgoing-neighbor source for [`OverlayOutSuccessors`].
enum OutBaseSource<'view> {
    /// Node-unique sorted, deduplicated row slice iterator.
    Unique(core::slice::Iter<'view, u32>),
    /// Parallel CSR edge walk with optional per-edge tombstone filtering.
    Parallel {
        /// Copy of the forward CSR graph for target/edge resolution.
        graph: CsrSnapshotGraph<'view, u32, u32>,
        /// Remaining outgoing edge slots for the expanded node.
        edges: oxgraph_csr::CsrOutEdges<u32>,
        /// Whether per-edge tombstone filtering is active.
        edge_tombstones: bool,
    },
}

impl OutBaseSource<'_> {
    /// Yields the next base target, applying per-edge tombstone filtering on the
    /// parallel path. Returns `None` when the base source is exhausted.
    fn next_raw(&mut self, overlay: &OverlayState) -> Option<u32> {
        match self {
            Self::Unique(iter) => iter.next().copied(),
            Self::Parallel {
                graph,
                edges,
                edge_tombstones,
            } => loop {
                let edge = edges.next()?;
                if *edge_tombstones && !overlay.edge_visible(edge.get()) {
                    continue;
                }
                return Some(EdgeTargetGraph::target(graph, edge).get());
            },
        }
    }
}

/// Successor iterator for [`OverlayOutView`] chaining base then overlay rows.
///
/// Yields only in-bound, node-visible targets so tombstoned nodes are neither
/// emitted nor expanded by the algo BFS.
///
/// # Performance
///
/// Advancing is amortized `O(1)`; the edge-tombstone parallel path is the
/// documented slow path (`perf: unspecified`).
#[expect(
    clippy::redundant_pub_crate,
    reason = "associated successor type of a crate-internal topology view"
)]
pub(crate) struct OverlayOutSuccessors<'view> {
    /// Base neighbor source (unique row or parallel walk).
    base: OutBaseSource<'view>,
    /// Overlay target row iterator chained after the base source.
    overlay: core::slice::Iter<'view, u32>,
    /// Live overlay for tombstone filtering.
    overlay_state: &'view OverlayState,
    /// Whether node tombstones must be filtered.
    check_nodes: bool,
    /// Exclusive node bound.
    node_count: u32,
}

impl OverlayOutSuccessors<'_> {
    /// Returns whether `node` is in bounds and node-visible.
    fn admit(&self, node: u32) -> bool {
        node < self.node_count && (!self.check_nodes || self.overlay_state.node_visible(node))
    }

    /// Pulls the next admitted base target, advancing past filtered slots.
    fn next_base(&mut self) -> Option<u32> {
        while let Some(candidate) = self.base.next_raw(self.overlay_state) {
            if self.admit(candidate) {
                return Some(candidate);
            }
        }
        None
    }
}

impl Iterator for OverlayOutSuccessors<'_> {
    type Item = u32;

    fn next(&mut self) -> Option<u32> {
        if let Some(target) = self.next_base() {
            return Some(target);
        }
        loop {
            let candidate = *self.overlay.next()?;
            if self.admit(candidate) {
                return Some(candidate);
            }
        }
    }
}

impl TopologyBase for OverlayOutView<'_> {
    type ElementId = u32;
    type RelationId = u32;
}

impl ContainsElement for OverlayOutView<'_> {
    fn contains_element(&self, element: u32) -> bool {
        (element as usize) < self.node_count
            && (!self.flags.check_nodes || self.overlay.node_visible(element))
    }
}

impl DenseElementIndex for OverlayOutView<'_> {
    fn element_bound(&self) -> usize {
        self.node_count
    }

    fn element_index(&self, element: u32) -> usize {
        element as usize
    }
}

impl ElementSuccessors for OverlayOutView<'_> {
    type Successors<'iter>
        = OverlayOutSuccessors<'iter>
    where
        Self: 'iter;

    fn element_successors(&self, element: u32) -> Self::Successors<'_> {
        let base = if self.flags.use_unique {
            OutBaseSource::Unique(self.unique.outgoing(element).iter())
        } else {
            OutBaseSource::Parallel {
                graph: self.forward.0,
                edges: OutgoingGraph::outgoing_edges(&self.forward.0, CsrNodeId::new(element)),
                edge_tombstones: self.overlay.has_edge_tombstones(),
            }
        };
        let overlay_row: &[u32] = if self.flags.merge_overlay {
            self.overlay.overlay_targets(element)
        } else {
            &[]
        };
        OverlayOutSuccessors {
            base,
            overlay: overlay_row.iter(),
            overlay_state: self.overlay,
            check_nodes: self.flags.check_nodes,
            node_count: u32::try_from(self.node_count).unwrap_or(u32::MAX),
        }
    }
}

/// Base incoming-neighbor source for [`OverlayInPredecessors`].
enum InBaseSource<'view> {
    /// Node-unique sorted, deduplicated row slice iterator.
    Unique(core::slice::Iter<'view, u32>),
    /// Parallel CSC predecessor walk with optional forward-edge tombstone check.
    Parallel {
        /// Remaining predecessor slots for the expanded node.
        preds: oxgraph_csc::CscPredecessors<'view, u32, u32>,
        /// Copy of the forward CSR graph for the forward-edge tombstone check.
        forward: CsrSnapshotGraph<'view, u32, u32>,
        /// The expanded node; predecessors are filtered against `p -> current`.
        current: u32,
        /// Whether the forward-edge tombstone check is active.
        edge_tombstones: bool,
    },
}

impl InBaseSource<'_> {
    /// Yields the next base source, applying the forward-edge tombstone check on
    /// the parallel path. Returns `None` when the base source is exhausted.
    fn next_raw(&mut self, overlay: &OverlayState) -> Option<u32> {
        match self {
            Self::Unique(iter) => iter.next().copied(),
            Self::Parallel {
                preds,
                forward,
                current,
                edge_tombstones,
            } => loop {
                let pred = preds.next()?.get();
                if *edge_tombstones && !has_visible_forward_edge(forward, overlay, pred, *current) {
                    continue;
                }
                return Some(pred);
            },
        }
    }
}

/// Predecessor iterator for [`OverlayInView`] chaining base then overlay rows.
///
/// Yields only in-bound, node-visible sources. The edge-tombstone slow path
/// keeps a predecessor `p` of `current` only when the forward edge
/// `p -> current` is not tombstoned.
///
/// # Performance
///
/// Advancing is amortized `O(1)`; the edge-tombstone path is the documented
/// slow path (`perf: unspecified`, worst case `O(degree)` per predecessor).
#[expect(
    clippy::redundant_pub_crate,
    reason = "associated predecessor type of a crate-internal topology view"
)]
pub(crate) struct OverlayInPredecessors<'view> {
    /// Base predecessor source (unique row or parallel walk).
    base: InBaseSource<'view>,
    /// Overlay source row iterator chained after the base source.
    overlay: core::slice::Iter<'view, u32>,
    /// Live overlay for tombstone filtering.
    overlay_state: &'view OverlayState,
    /// Whether node tombstones must be filtered.
    check_nodes: bool,
    /// Exclusive node bound.
    node_count: u32,
}

impl OverlayInPredecessors<'_> {
    /// Returns whether `node` is in bounds and node-visible.
    fn admit(&self, node: u32) -> bool {
        node < self.node_count && (!self.check_nodes || self.overlay_state.node_visible(node))
    }

    /// Pulls the next admitted base source, advancing past filtered slots.
    fn next_base(&mut self) -> Option<u32> {
        while let Some(candidate) = self.base.next_raw(self.overlay_state) {
            if self.admit(candidate) {
                return Some(candidate);
            }
        }
        None
    }
}

impl Iterator for OverlayInPredecessors<'_> {
    type Item = u32;

    fn next(&mut self) -> Option<u32> {
        if let Some(source) = self.next_base() {
            return Some(source);
        }
        loop {
            let candidate = *self.overlay.next()?;
            if self.admit(candidate) {
                return Some(candidate);
            }
        }
    }
}

impl TopologyBase for OverlayInView<'_> {
    type ElementId = u32;
    type RelationId = u32;
}

impl ContainsElement for OverlayInView<'_> {
    fn contains_element(&self, element: u32) -> bool {
        (element as usize) < self.node_count
            && (!self.flags.check_nodes || self.overlay.node_visible(element))
    }
}

impl DenseElementIndex for OverlayInView<'_> {
    fn element_bound(&self) -> usize {
        self.node_count
    }

    fn element_index(&self, element: u32) -> usize {
        element as usize
    }
}

impl ElementPredecessors for OverlayInView<'_> {
    type Predecessors<'iter>
        = OverlayInPredecessors<'iter>
    where
        Self: 'iter;

    fn element_predecessors(&self, element: u32) -> Self::Predecessors<'_> {
        let base = if self.flags.use_unique {
            InBaseSource::Unique(self.unique.incoming(element).iter())
        } else {
            InBaseSource::Parallel {
                preds: self.inbound.0.element_predecessors(CscNodeId::new(element)),
                forward: self.forward.0,
                current: element,
                edge_tombstones: self.overlay.has_edge_tombstones(),
            }
        };
        let overlay_row: &[u32] = if self.flags.merge_overlay {
            self.overlay.overlay_sources(element)
        } else {
            &[]
        };
        OverlayInPredecessors {
            base,
            overlay: overlay_row.iter(),
            overlay_state: self.overlay,
            check_nodes: self.flags.check_nodes,
            node_count: u32::try_from(self.node_count).unwrap_or(u32::MAX),
        }
    }
}
