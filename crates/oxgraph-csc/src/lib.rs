//! Borrowed compressed-sparse-column (inbound) graph views.
//!
//! Incoming adjacency is stored in Postgres-owned OXGTOPO sections (`0x0201` /
//! `0x0202`). The physical layout matches CSR on transposed edges; this crate
//! is separate from [`oxgraph_csr`] so forward and inbound views cannot be
//! mixed at the type level.
//!
//! # Performance
//!
//! Opening a snapshot-backed view is `O(s + n + m)` once per engine load.
#![no_std]

use core::fmt;

use oxgraph_csr::{CsrEdgeId, CsrGraph, CsrNodeId, CsrSnapshotError, CsrSnapshotIndex};
use oxgraph_graph::{ElementPredecessors, OutgoingNeighborsGraph, TopologyCounts};
use oxgraph_snapshot::Snapshot;
use oxgraph_topology::TopologyBase;

/// Postgres-owned inbound offsets section (`0x0201`).
pub const SNAPSHOT_KIND_PG_INBOUND_OFFSETS_U32: u32 = 0x0201;

/// Postgres-owned inbound targets section (`0x0202`).
pub const SNAPSHOT_KIND_PG_INBOUND_TARGETS_U32: u32 = 0x0202;

/// Dense node id in an inbound CSC view.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CscNodeId(pub u32);

/// Errors while opening inbound CSC sections from a snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CscSnapshotError {
    /// Underlying CSR layout validation failed.
    Layout(CsrSnapshotError<u32, u32>),
}

impl fmt::Display for CscSnapshotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Layout(error) => write!(f, "inbound layout error: {error}"),
        }
    }
}

impl core::error::Error for CscSnapshotError {}

impl From<CsrSnapshotError<u32, u32>> for CscSnapshotError {
    fn from(error: CsrSnapshotError<u32, u32>) -> Self {
        Self::Layout(error)
    }
}

/// Snapshot-backed inbound CSC view for `u32` node and edge indices.
///
/// # Performance
///
/// `perf: unspecified`; this alias carries no runtime cost.
pub type CscInnerGraph<'view> = CsrGraph<
    'view,
    u32,
    u32,
    <u32 as CsrSnapshotIndex>::LittleEndianWord,
    <u32 as CsrSnapshotIndex>::LittleEndianWord,
>;

/// Borrowed inbound adjacency (transpose-CSR / CSC layout).
///
/// # Performance
///
/// Per-node predecessor enumeration is `O(k)` for `k` incoming edges.
#[derive(Clone, Copy, Debug)]
pub struct CscSnapshotGraph<'view>(CscInnerGraph<'view>);

impl<'view> CscSnapshotGraph<'view> {
    /// Opens inbound sections from a validated [`Snapshot`].
    ///
    /// # Errors
    ///
    /// Returns [`CscSnapshotError`] when sections are missing or fail validation.
    ///
    /// # Performance
    ///
    /// This function is `O(s + n + m)`.
    pub fn from_snapshot(snapshot: &Snapshot<'view>) -> Result<Self, CscSnapshotError> {
        Self::from_snapshot_with_kinds(
            snapshot,
            SNAPSHOT_KIND_PG_INBOUND_OFFSETS_U32,
            SNAPSHOT_KIND_PG_INBOUND_TARGETS_U32,
        )
    }

    /// Opens inbound sections using explicit snapshot section kinds.
    ///
    /// # Errors
    ///
    /// Returns [`CscSnapshotError`] when sections are missing or fail validation.
    ///
    /// # Performance
    ///
    /// This function is `O(s + n + m)`.
    pub fn from_snapshot_with_kinds(
        snapshot: &Snapshot<'view>,
        offsets_kind: u32,
        targets_kind: u32,
    ) -> Result<Self, CscSnapshotError> {
        let offsets_section = snapshot
            .section(offsets_kind)
            .ok_or(CsrSnapshotError::MissingOffsets)?;
        let targets_section = snapshot
            .section(targets_kind)
            .ok_or(CsrSnapshotError::MissingTargets)?;

        let offsets: &'view [<u32 as CsrSnapshotIndex>::LittleEndianWord] = offsets_section
            .try_as_slice()
            .map_err(CsrSnapshotError::OffsetsView)?;
        let targets: &'view [<u32 as CsrSnapshotIndex>::LittleEndianWord] = targets_section
            .try_as_slice()
            .map_err(CsrSnapshotError::TargetsView)?;

        if offsets.is_empty() {
            return Err(CsrSnapshotError::OffsetsEmpty.into());
        }

        let node_count_usize = offsets.len() - 1;
        let node_count =
            u32::try_from(node_count_usize).map_err(|_| CsrSnapshotError::NodeCountOverflow {
                offsets_len: offsets.len(),
            })?;

        let inner = CscInnerGraph::validate(node_count, offsets, targets)
            .map_err(|error| CscSnapshotError::Layout(CsrSnapshotError::Csr(error)))?;
        Ok(Self(inner))
    }

    /// Returns the inner CSR graph backing this inbound view.
    #[must_use]
    pub const fn inner(&self) -> &CscInnerGraph<'view> {
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
    #[must_use]
    pub fn predecessors(&self, target: u32) -> impl ExactSizeIterator<Item = u32> + '_ {
        self.0.outgoing_neighbors(CsrNodeId(target)).map(|id| id.0)
    }

    /// Walks predecessor node ids for `target` via the CSC source slice.
    ///
    /// Stops early when `visit` returns `true`.
    ///
    /// # Performance
    ///
    /// This method is `O(k)` for `k` predecessors with no iterator adapters.
    pub fn for_each_predecessor(&self, target: u32, mut visit: impl FnMut(u32) -> bool) -> bool {
        self.0
            .for_each_out_target(CsrNodeId(target), |id| visit(id.0))
    }
}

/// Iterator over predecessor node ids in one inbound row.
pub struct CscPredecessors<'view> {
    /// Underlying CSR out-neighbor iterator for the transposed inbound row.
    inner: <CscInnerGraph<'view> as OutgoingNeighborsGraph>::OutNeighbors<'view>,
}

impl Iterator for CscPredecessors<'_> {
    type Item = CscNodeId;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|id| CscNodeId(id.0))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl ExactSizeIterator for CscPredecessors<'_> {}

impl TopologyBase for CscSnapshotGraph<'_> {
    type ElementId = CscNodeId;
    type RelationId = CsrEdgeId<u32>;
}

impl TopologyCounts for CscSnapshotGraph<'_> {
    fn element_count(&self) -> usize {
        self.0.element_count()
    }

    fn relation_count(&self) -> usize {
        self.0.relation_count()
    }
}

impl ElementPredecessors for CscSnapshotGraph<'_> {
    type Predecessors<'view>
        = CscPredecessors<'view>
    where
        Self: 'view;

    fn element_predecessors(&self, element: CscNodeId) -> Self::Predecessors<'_> {
        CscPredecessors {
            inner: self.0.outgoing_neighbors(CsrNodeId(element.0)),
        }
    }
}

#[cfg(kani)]
mod proofs;
