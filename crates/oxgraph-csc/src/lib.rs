//! Borrowed compressed-sparse-column (inbound) graph views.
//!
//! Incoming adjacency is stored as CSR-on-transposed-edges in OXGTOPO sections.
//! This crate is separate from [`oxgraph_csr`] so forward and inbound views
//! cannot be mixed at the type level. The view is storage-agnostic: it does not
//! own any section-kind constants and reads whatever offsets/targets kinds the
//! caller supplies through [`CscSnapshotGraph::from_snapshot_with_kinds`]. The
//! storage layer that persists the inbound index (for example the Postgres
//! engine) owns the section-kind constants and the section version.
//!
//! # Performance
//!
//! Opening a snapshot-backed view is `O(s + n + m)` once per engine load.
#![no_std]

use core::fmt;

use oxgraph_csr::{CsrEdgeId, CsrGraph, CsrNodeId, CsrSnapshotError, CsrSnapshotIndex};
use oxgraph_graph::{ElementPredecessors, OutgoingNeighborsGraph, TopologyCounts};
use oxgraph_layout_util::{NodeAxis, SnapshotWidth};
use oxgraph_snapshot::Snapshot;
use oxgraph_topology::TopologyBase;

/// Dense node id in an inbound CSC view.
///
/// Alias of the shared [`LocalId`](oxgraph_layout_util::LocalId) branded by the
/// node axis, parameterized over the node index width `N`.
///
/// # Performance
///
/// Copying, comparing, ordering, hashing, and debug-formatting are `O(1)`.
pub type CscNodeId<N = u32> = oxgraph_layout_util::LocalId<NodeAxis, N>;

/// Errors while opening inbound CSC sections from a snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CscSnapshotError<N = u32, E = u32> {
    /// Underlying CSR layout validation failed.
    Layout(CsrSnapshotError<N, E>),
}

impl<N: fmt::Display, E: fmt::Display> fmt::Display for CscSnapshotError<N, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Layout(error) => write!(f, "inbound layout error: {error}"),
        }
    }
}

impl<N, E> core::error::Error for CscSnapshotError<N, E>
where
    N: fmt::Debug + fmt::Display,
    E: fmt::Debug + fmt::Display,
{
}

impl<N, E> From<CsrSnapshotError<N, E>> for CscSnapshotError<N, E> {
    fn from(error: CsrSnapshotError<N, E>) -> Self {
        Self::Layout(error)
    }
}

/// Snapshot-backed inbound CSC view for node width `N` and edge width `E`.
///
/// # Performance
///
/// `perf: unspecified`; this alias carries no runtime cost.
pub type CscInnerGraph<'view, N = u32, E = u32> = CsrGraph<
    'view,
    N,
    E,
    <E as SnapshotWidth>::LittleEndianWord,
    <N as SnapshotWidth>::LittleEndianWord,
>;

/// Borrowed inbound adjacency (transpose-CSR / CSC layout).
///
/// `N` is the node index width and `E` is the edge index width; both default to
/// `u32` for the common engine configuration.
///
/// # Performance
///
/// Per-node predecessor enumeration is `O(k)` for `k` incoming edges.
#[derive(Clone, Copy)]
pub struct CscSnapshotGraph<'view, N = u32, E = u32>(CscInnerGraph<'view, N, E>)
where
    N: CsrSnapshotIndex,
    E: CsrSnapshotIndex;

impl<N, E> fmt::Debug for CscSnapshotGraph<'_, N, E>
where
    N: CsrSnapshotIndex,
    E: CsrSnapshotIndex,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CscSnapshotGraph")
            .field("node_count", &self.node_count())
            .field("relation_count", &self.relation_count())
            .finish()
    }
}

impl<'view, N, E> CscSnapshotGraph<'view, N, E>
where
    N: CsrSnapshotIndex,
    E: CsrSnapshotIndex,
{
    /// Opens inbound sections using explicit snapshot section kinds.
    ///
    /// Reuses [`CsrGraph::from_snapshot_with_kinds`] over the transposed-edge
    /// layout, so callers select whichever band their storage layer reserves
    /// for the inbound index. CSC owns no section-kind constants.
    ///
    /// # Errors
    ///
    /// Returns [`CscSnapshotError`] when sections are missing, have the wrong
    /// version, or fail validation.
    ///
    /// # Performance
    ///
    /// This function is `O(s + n + m)`.
    pub fn from_snapshot_with_kinds(
        snapshot: &Snapshot<'view>,
        offsets_kind: u32,
        targets_kind: u32,
        version: u32,
    ) -> Result<Self, CscSnapshotError<N, E>> {
        let inner = CscInnerGraph::<'view, N, E>::from_snapshot_with_kinds(
            snapshot,
            offsets_kind,
            targets_kind,
            version,
        )?;
        Ok(Self(inner))
    }

    /// Returns the inner CSR graph backing this inbound view.
    #[must_use]
    pub const fn inner(&self) -> &CscInnerGraph<'view, N, E> {
        &self.0
    }

    /// Returns the number of nodes in this inbound view.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub fn node_count(&self) -> usize {
        self.0.element_count()
    }

    /// Returns the number of edges in this inbound view.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub fn relation_count(&self) -> usize {
        self.0.relation_count()
    }

    /// Returns an iterator over predecessor node ids for `target`.
    ///
    /// # Performance
    ///
    /// This method is `O(1)` to create and `O(k)` to yield `k` predecessors.
    pub fn predecessors(
        &self,
        target: CscNodeId<N>,
    ) -> impl ExactSizeIterator<Item = CscNodeId<N>> + '_ {
        self.0
            .outgoing_neighbors(CsrNodeId::new(target.get()))
            .map(|id| CscNodeId::new(id.get()))
    }

    /// Walks predecessor node ids for `target` via the CSC source slice.
    ///
    /// Stops early when `visit` returns `true`.
    ///
    /// # Performance
    ///
    /// This method is `O(k)` for `k` predecessors with no iterator adapters.
    pub fn for_each_predecessor(
        &self,
        target: CscNodeId<N>,
        mut visit: impl FnMut(CscNodeId<N>) -> bool,
    ) -> bool {
        self.0
            .for_each_out_target(CsrNodeId::new(target.get()), |id| {
                visit(CscNodeId::new(id.get()))
            })
    }
}

/// Iterator over predecessor node ids in one inbound row.
pub struct CscPredecessors<'view, N = u32, E = u32>
where
    N: CsrSnapshotIndex + 'view,
    E: CsrSnapshotIndex + 'view,
{
    /// Underlying CSR out-neighbor iterator for the transposed inbound row.
    inner: <CscInnerGraph<'view, N, E> as OutgoingNeighborsGraph>::OutNeighbors<'view>,
}

impl<N, E> Iterator for CscPredecessors<'_, N, E>
where
    N: CsrSnapshotIndex,
    E: CsrSnapshotIndex,
{
    type Item = CscNodeId<N>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|id| CscNodeId::new(id.get()))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<N, E> ExactSizeIterator for CscPredecessors<'_, N, E>
where
    N: CsrSnapshotIndex,
    E: CsrSnapshotIndex,
{
}

impl<N, E> TopologyBase for CscSnapshotGraph<'_, N, E>
where
    N: CsrSnapshotIndex,
    E: CsrSnapshotIndex,
{
    type ElementId = CscNodeId<N>;
    type RelationId = CsrEdgeId<E>;
}

impl<N, E> TopologyCounts for CscSnapshotGraph<'_, N, E>
where
    N: CsrSnapshotIndex,
    E: CsrSnapshotIndex,
{
    fn element_count(&self) -> usize {
        self.0.element_count()
    }

    fn relation_count(&self) -> usize {
        self.0.relation_count()
    }
}

impl<N, E> ElementPredecessors for CscSnapshotGraph<'_, N, E>
where
    N: CsrSnapshotIndex,
    E: CsrSnapshotIndex,
{
    type Predecessors<'view>
        = CscPredecessors<'view, N, E>
    where
        Self: 'view;

    fn element_predecessors(&self, element: CscNodeId<N>) -> Self::Predecessors<'_> {
        CscPredecessors {
            inner: self.0.outgoing_neighbors(CsrNodeId::new(element.get())),
        }
    }
}

#[cfg(kani)]
mod proofs;
