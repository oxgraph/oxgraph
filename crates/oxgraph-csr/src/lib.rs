//! Borrowed compressed-sparse-row graph views.
//!
//! `oxgraph-csr` provides the first concrete graph layout for the substrate. A
//! [`CsrGraph`] borrows validated CSR offset and target slices and implements
//! storage-agnostic graph traits from `oxgraph-graph`.
//!
//! CSR is optimized for outgoing traversal. Incoming traversal requires a CSC
//! index or another reverse index and is intentionally not implemented here.
#![no_std]

#[cfg(kani)]
extern crate kani;

#[cfg(kani)]
mod proofs;

use core::fmt;

use oxgraph_graph::{
    ContainsElement, ContainsRelation, EdgeTargetGraph, ElementIndex, ElementSuccessors,
    GraphCounts, OutgoingEdgeCount, OutgoingGraph, RelationIndex, TopologyBase, TopologyCounts,
};
use oxgraph_snapshot::{SectionViewError, Snapshot};
use zerocopy::byteorder::{LE, U32};

/// Section kind for a CSR offsets array stored in an `oxgraph-snapshot`.
///
/// The payload is a sequence of unaligned little-endian `u32` words of
/// length `node_count + 1`. The CSR view derives `node_count` from this
/// length; no separate metadata section is required.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub const SNAPSHOT_KIND_CSR_OFFSETS: u32 = 0x0001;

/// Section kind for a CSR targets array stored in an `oxgraph-snapshot`.
///
/// The payload is a sequence of unaligned little-endian `u32` words whose
/// length equals the final value of the CSR offsets array.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub const SNAPSHOT_KIND_CSR_TARGETS: u32 = 0x0002;

/// Integer word usable in borrowed CSR sections.
///
/// The default in-memory representation uses `u32`. Snapshot-backed views can
/// use unaligned little-endian [`U32<LE>`] words and still implement the same
/// graph traits without copying section data.
///
/// # Performance
///
/// Reading a word is expected to be `O(1)`.
pub trait CsrWord: Copy {
    /// Returns this CSR word as a host-endian `u32`.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    fn get(self) -> u32;
}

impl CsrWord for u32 {
    fn get(self) -> u32 {
        self
    }
}

impl CsrWord for U32<LE> {
    fn get(self) -> u32 {
        self.get()
    }
}

/// Local node ID for [`CsrGraph`].
///
/// Values are dense `u32` handles in `0..node_count` for one validated CSR view.
/// They are topology-local IDs and are not stable across rebuilding or
/// compaction unless a higher layer defines that contract.
///
/// # Performance
///
/// Copying, comparing, ordering, hashing, and debug-formatting are `O(1)`.
#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CsrNodeId(pub u32);

impl fmt::Debug for CsrNodeId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("CsrNodeId").field(&self.0).finish()
    }
}

/// Local edge ID for [`CsrGraph`].
///
/// Values are dense `u32` handles into the flat CSR target array. They are
/// topology-local IDs and are not stable across sorting, rebuilding, or
/// compaction unless a higher layer defines that contract.
///
/// # Performance
///
/// Copying, comparing, ordering, hashing, and debug-formatting are `O(1)`.
#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CsrEdgeId(pub u32);

impl fmt::Debug for CsrEdgeId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("CsrEdgeId").field(&self.0).finish()
    }
}

/// Borrowed compressed-sparse-row graph view.
///
/// The graph stores outgoing adjacency using `offsets[node]..offsets[node + 1]`
/// ranges into the flat `targets` slice. The view borrows both slices and does
/// not allocate.
///
/// # Performance
///
/// Creating a validated view is `O(n + m)` for `n` nodes and `m` edges because
/// validation checks monotonic offsets and target bounds. Outgoing traversal for
/// one node is `O(1)` to create and `O(k)` to yield `k` outgoing edges.
#[derive(Clone, Copy, Debug)]
pub struct CsrGraph<'view, Word = u32> {
    /// Number of nodes in the graph.
    node_count: u32,
    /// CSR offsets with length `node_count + 1`.
    offsets: &'view [Word],
    /// Flat outgoing target node IDs.
    targets: &'view [Word],
}

impl<'view, Word> CsrGraph<'view, Word>
where
    Word: CsrWord,
{
    /// Validates borrowed CSR sections and returns a graph view.
    ///
    /// # Errors
    ///
    /// Returns [`CsrError`] when offsets have the wrong length, offsets are not
    /// monotonic, the final offset does not match `targets.len()`, a target is
    /// out of range, or a `u32` count cannot be represented as `usize` on the
    /// current target.
    ///
    /// # Performance
    ///
    /// Validation is `O(n + m)` for `n` nodes and `m` edges.
    pub fn validate(
        node_count: u32,
        offsets: &'view [Word],
        targets: &'view [Word],
    ) -> Result<Self, CsrError> {
        let expected_offsets = u32_to_usize(node_count)?
            .checked_add(1)
            .ok_or(CsrError::OffsetLengthOverflow { node_count })?;

        if offsets.len() != expected_offsets {
            return Err(CsrError::OffsetLength {
                expected: expected_offsets,
                actual: offsets.len(),
            });
        }

        let mut previous = 0;
        for (index, offset_word) in offsets.iter().copied().enumerate() {
            let offset = offset_word.get();
            if index == 0 && offset != 0 {
                return Err(CsrError::FirstOffset { actual: offset });
            }
            if offset < previous {
                return Err(CsrError::NonMonotonicOffset {
                    index,
                    previous,
                    actual: offset,
                });
            }
            previous = offset;
        }

        let final_offset = offsets[offsets.len() - 1].get();
        let final_offset_usize = u32_to_usize(final_offset)?;
        if final_offset_usize != targets.len() {
            return Err(CsrError::FinalOffset {
                final_offset,
                target_len: targets.len(),
            });
        }

        for (index, target_word) in targets.iter().copied().enumerate() {
            let target = target_word.get();
            if target >= node_count {
                return Err(CsrError::TargetOutOfRange {
                    index,
                    target,
                    node_count,
                });
            }
        }

        Ok(Self {
            node_count,
            offsets,
            targets,
        })
    }

    /// Returns the borrowed CSR offset slice.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub const fn offsets(&self) -> &'view [Word] {
        self.offsets
    }

    /// Returns the borrowed CSR target slice.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub const fn targets(&self) -> &'view [Word] {
        self.targets
    }

    /// Returns whether `node` is valid in this CSR view.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub const fn contains_node(&self, node: CsrNodeId) -> bool {
        node.0 < self.node_count
    }

    /// Returns whether `edge` is valid in this CSR view.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub fn contains_edge(&self, edge: CsrEdgeId) -> bool {
        match usize::try_from(edge.0) {
            Ok(index) => index < self.targets.len(),
            Err(_error) => false,
        }
    }

    /// Returns the target node for `edge` when `edge` is valid in this view.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub fn try_target(&self, edge: CsrEdgeId) -> Option<CsrNodeId> {
        self.contains_edge(edge).then(|| self.target_node(edge))
    }

    /// Returns the target node for a CSR edge slot.
    ///
    /// # Performance
    ///
    /// This method is `O(1)` for valid edge IDs from this view.
    fn target_node(&self, edge: CsrEdgeId) -> CsrNodeId {
        CsrNodeId(self.targets[u32_to_usize_lossless(edge.0)].get())
    }

    /// Returns the start and end edge slots for a node.
    ///
    /// # Performance
    ///
    /// This method is `O(1)` for valid node IDs from this view.
    fn outgoing_range(&self, node: CsrNodeId) -> (u32, u32) {
        let index = u32_to_usize_lossless(node.0);
        (self.offsets[index].get(), self.offsets[index + 1].get())
    }
}

impl<'view> CsrGraph<'view, U32<LE>> {
    /// Builds a snapshot-backed CSR view from a validated [`Snapshot`].
    ///
    /// Reads the [`SNAPSHOT_KIND_CSR_OFFSETS`] and [`SNAPSHOT_KIND_CSR_TARGETS`]
    /// sections, derives `node_count` from `offsets.len() - 1`, and runs
    /// CSR-shape validation. The returned view borrows directly from the
    /// snapshot's byte slice — no copying.
    ///
    /// # Errors
    ///
    /// Returns [`CsrSnapshotError`] when either section is missing,
    /// misaligned, of invalid length, or fails CSR validation.
    ///
    /// # Performance
    ///
    /// This function is `O(s + n + m)` for `s` snapshot sections, `n` graph
    /// nodes, and `m` graph edges.
    pub fn from_snapshot(snapshot: &Snapshot<'view>) -> Result<Self, CsrSnapshotError> {
        let offsets_section = snapshot
            .section(SNAPSHOT_KIND_CSR_OFFSETS)
            .ok_or(CsrSnapshotError::MissingOffsets)?;
        let targets_section = snapshot
            .section(SNAPSHOT_KIND_CSR_TARGETS)
            .ok_or(CsrSnapshotError::MissingTargets)?;

        let offsets: &'view [U32<LE>] = offsets_section
            .try_as_slice()
            .map_err(CsrSnapshotError::OffsetsView)?;
        let targets: &'view [U32<LE>] = targets_section
            .try_as_slice()
            .map_err(CsrSnapshotError::TargetsView)?;

        if offsets.is_empty() {
            return Err(CsrSnapshotError::OffsetsEmpty);
        }

        let node_count_usize = offsets.len() - 1;
        let node_count = u32::try_from(node_count_usize).map_err(|_error| {
            CsrSnapshotError::NodeCountOverflow {
                offsets_len: offsets.len(),
            }
        })?;

        Ok(Self::validate(node_count, offsets, targets)?)
    }
}

impl<Word> TopologyBase for CsrGraph<'_, Word>
where
    Word: CsrWord,
{
    type ElementId = CsrNodeId;
    type RelationId = CsrEdgeId;
}

impl<Word> TopologyCounts for CsrGraph<'_, Word>
where
    Word: CsrWord,
{
    fn element_count(&self) -> usize {
        u32_to_usize_lossless(self.node_count)
    }

    fn relation_count(&self) -> usize {
        self.targets.len()
    }
}

impl<Word> GraphCounts for CsrGraph<'_, Word> where Word: CsrWord {}

impl<Word> ElementIndex for CsrGraph<'_, Word>
where
    Word: CsrWord,
{
    fn element_bound(&self) -> usize {
        u32_to_usize_lossless(self.node_count)
    }

    fn element_index(&self, element: CsrNodeId) -> usize {
        u32_to_usize_lossless(element.0)
    }
}

impl<Word> RelationIndex for CsrGraph<'_, Word>
where
    Word: CsrWord,
{
    fn relation_bound(&self) -> usize {
        self.targets.len()
    }

    fn relation_index(&self, relation: CsrEdgeId) -> usize {
        u32_to_usize_lossless(relation.0)
    }
}

impl<Word> ContainsElement for CsrGraph<'_, Word>
where
    Word: CsrWord,
{
    fn contains_element(&self, element: CsrNodeId) -> bool {
        self.contains_node(element)
    }
}

impl<Word> ContainsRelation for CsrGraph<'_, Word>
where
    Word: CsrWord,
{
    fn contains_relation(&self, relation: CsrEdgeId) -> bool {
        self.contains_edge(relation)
    }
}

impl<Word> EdgeTargetGraph for CsrGraph<'_, Word>
where
    Word: CsrWord,
{
    fn target(&self, edge: CsrEdgeId) -> CsrNodeId {
        self.target_node(edge)
    }
}

impl<Word> OutgoingGraph for CsrGraph<'_, Word>
where
    Word: CsrWord,
{
    type OutEdges<'view>
        = CsrOutEdges
    where
        Self: 'view;

    fn outgoing_edges(&self, node: CsrNodeId) -> Self::OutEdges<'_> {
        let (next, end) = self.outgoing_range(node);
        CsrOutEdges { next, end }
    }
}

impl<Word> OutgoingEdgeCount for CsrGraph<'_, Word>
where
    Word: CsrWord,
{
    fn out_degree(&self, node: CsrNodeId) -> usize {
        let (start, end) = self.outgoing_range(node);
        u32_to_usize_lossless(end - start)
    }
}

impl<Word> ElementSuccessors for CsrGraph<'_, Word>
where
    Word: CsrWord,
{
    type Successors<'view>
        = CsrOutNeighbors<'view, Word>
    where
        Self: 'view;

    fn element_successors(&self, node: CsrNodeId) -> Self::Successors<'_> {
        let (start, end) = self.outgoing_range(node);
        CsrOutNeighbors {
            targets: self.targets[u32_to_usize_lossless(start)..u32_to_usize_lossless(end)].iter(),
        }
    }
}

/// Iterator over outgoing CSR edge slots.
///
/// # Performance
///
/// Advancing the iterator is `O(1)`.
#[derive(Clone, Debug)]
pub struct CsrOutEdges {
    /// Next edge slot to yield.
    next: u32,
    /// Exclusive end edge slot.
    end: u32,
}

impl Iterator for CsrOutEdges {
    type Item = CsrEdgeId;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next == self.end {
            return None;
        }

        let edge = CsrEdgeId(self.next);
        self.next += 1;
        Some(edge)
    }
}

impl ExactSizeIterator for CsrOutEdges {
    fn len(&self) -> usize {
        u32_to_usize_lossless(self.end - self.next)
    }
}

/// Iterator over outgoing CSR target nodes.
///
/// This iterator borrows the validated target slice for one node's outgoing
/// range and yields target node IDs directly. Parallel edges and self-loops are
/// preserved because each target entry is yielded once in CSR order.
///
/// # Performance
///
/// Advancing the iterator is `O(1)` and performs no allocation.
#[derive(Clone, Debug)]
pub struct CsrOutNeighbors<'view, Word> {
    /// Remaining target words in the outgoing range.
    targets: core::slice::Iter<'view, Word>,
}

impl<Word> Iterator for CsrOutNeighbors<'_, Word>
where
    Word: CsrWord,
{
    type Item = CsrNodeId;

    fn next(&mut self) -> Option<Self::Item> {
        self.targets.next().map(|word| CsrNodeId(word.get()))
    }
}

impl<Word> ExactSizeIterator for CsrOutNeighbors<'_, Word>
where
    Word: CsrWord,
{
    fn len(&self) -> usize {
        self.targets.len()
    }
}

/// CSR validation error.
///
/// # Performance
///
/// `perf: unspecified`; errors are returned only from validation paths.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CsrError {
    /// `node_count + 1` overflowed `usize`.
    OffsetLengthOverflow {
        /// Node count that could not be converted to an offset length.
        node_count: u32,
    },
    /// Offset slice length does not equal `node_count + 1`.
    OffsetLength {
        /// Expected offset length.
        expected: usize,
        /// Actual offset length.
        actual: usize,
    },
    /// The first CSR offset was not zero.
    FirstOffset {
        /// Actual first offset.
        actual: u32,
    },
    /// Offsets were not monotonically increasing.
    NonMonotonicOffset {
        /// Offset index where monotonicity failed.
        index: usize,
        /// Previous offset value.
        previous: u32,
        /// Actual offset value at `index`.
        actual: u32,
    },
    /// Final offset does not match target slice length.
    FinalOffset {
        /// Final offset value.
        final_offset: u32,
        /// Target slice length.
        target_len: usize,
    },
    /// Target node ID is outside `0..node_count`.
    TargetOutOfRange {
        /// Target slice index containing the bad value.
        index: usize,
        /// Bad target node ID.
        target: u32,
        /// Number of nodes in the graph.
        node_count: u32,
    },
    /// A `u32` value could not be represented as `usize` on this target.
    UsizeOverflow {
        /// Value that could not be represented as `usize`.
        value: u32,
    },
}

impl fmt::Display for CsrError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OffsetLengthOverflow { node_count } => {
                write!(
                    formatter,
                    "offset length overflow for node count {node_count}"
                )
            }
            Self::OffsetLength { expected, actual } => write!(
                formatter,
                "invalid CSR offset length: expected {expected}, got {actual}"
            ),
            Self::FirstOffset { actual } => {
                write!(formatter, "first CSR offset must be 0, got {actual}")
            }
            Self::NonMonotonicOffset {
                index,
                previous,
                actual,
            } => write!(
                formatter,
                "CSR offset at index {index} is not monotonic: previous {previous}, got {actual}"
            ),
            Self::FinalOffset {
                final_offset,
                target_len,
            } => write!(
                formatter,
                "final CSR offset {final_offset} does not match target length {target_len}"
            ),
            Self::TargetOutOfRange {
                index,
                target,
                node_count,
            } => write!(
                formatter,
                "CSR target at index {index} is out of range: target {target}, node count {node_count}"
            ),
            Self::UsizeOverflow { value } => {
                write!(formatter, "u32 value {value} does not fit usize")
            }
        }
    }
}

impl core::error::Error for CsrError {}

/// Error returned when a snapshot cannot be opened as a CSR graph.
///
/// # Performance
///
/// `perf: unspecified`; errors are returned only from snapshot-bound paths.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CsrSnapshotError {
    /// The snapshot has no [`SNAPSHOT_KIND_CSR_OFFSETS`] section.
    MissingOffsets,
    /// The snapshot has no [`SNAPSHOT_KIND_CSR_TARGETS`] section.
    MissingTargets,
    /// The CSR offsets section payload could not be borrowed as `[U32<LE>]`.
    OffsetsView(SectionViewError),
    /// The CSR targets section payload could not be borrowed as `[U32<LE>]`.
    TargetsView(SectionViewError),
    /// The CSR offsets section is empty; CSR requires at least one entry
    /// for the n-plus-one layout.
    OffsetsEmpty,
    /// The derived node count would not fit in `u32`.
    NodeCountOverflow {
        /// Length of the offsets section.
        offsets_len: usize,
    },
    /// CSR-shape validation failed on the borrowed sections.
    Csr(CsrError),
}

impl fmt::Display for CsrSnapshotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingOffsets => formatter.write_str("snapshot has no CSR offsets section"),
            Self::MissingTargets => formatter.write_str("snapshot has no CSR targets section"),
            Self::OffsetsView(error) => write!(
                formatter,
                "CSR offsets section cannot be borrowed as little-endian u32: {error}"
            ),
            Self::TargetsView(error) => write!(
                formatter,
                "CSR targets section cannot be borrowed as little-endian u32: {error}"
            ),
            Self::OffsetsEmpty => formatter.write_str("CSR offsets section is empty"),
            Self::NodeCountOverflow { offsets_len } => write!(
                formatter,
                "derived node count from offsets length {offsets_len} does not fit u32"
            ),
            Self::Csr(error) => write!(formatter, "CSR validation failed: {error}"),
        }
    }
}

impl core::error::Error for CsrSnapshotError {}

impl From<CsrError> for CsrSnapshotError {
    fn from(error: CsrError) -> Self {
        Self::Csr(error)
    }
}

/// Converts a `u32` to `usize` and reports overflow on narrow targets.
///
/// # Performance
///
/// This function is `O(1)`.
fn u32_to_usize(value: u32) -> Result<usize, CsrError> {
    usize::try_from(value).map_err(|_| CsrError::UsizeOverflow { value })
}

/// Converts a previously validated `u32` to `usize`.
///
/// # Panics
///
/// Panics via `unreachable!()` only on a target where `usize` is narrower
/// than `u32` AND the caller has supplied a value that was not first vetted
/// by [`u32_to_usize`]. All in-tree callers vet through [`CsrGraph::validate`]
/// (which fails open-time with [`CsrError::UsizeOverflow`] before any
/// `_lossless` call), so on supported targets this branch is dead.
///
/// # Performance
///
/// This function is `O(1)`.
fn u32_to_usize_lossless(value: u32) -> usize {
    match usize::try_from(value) {
        Ok(converted) => converted,
        Err(_error) => unreachable!("validated CSR u32 value must fit usize"),
    }
}
