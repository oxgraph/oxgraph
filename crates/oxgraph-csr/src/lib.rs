//! Borrowed compressed-sparse-row graph views.
//!
//! `oxgraph-csr` provides the first concrete graph layout for the substrate. A
//! [`CsrGraph`] borrows validated CSR offset and target slices and implements
//! storage-agnostic graph traits from `oxgraph-graph`.
//!
//! CSR is optimized for outgoing traversal. Incoming traversal requires a CSC
//! index or another reverse index and is intentionally not implemented here.
//!
//! # Optional builder
//!
//! The `build` feature enables append-only [`build::GraphBuilder`] and
//! [`build::WeightedGraphBuilder`] types, plus snapshot export helpers, in
//! the [`build`] submodule. The additional `build-property-arrow` feature
//! enables property-snapshot export via `oxgraph-property`.
//!
//! # Snapshot section kinds
//!
//! | Family | Description |
//! | ------ | ----------- |
//! | `CSR_OFFSETS_*` | CSR offsets array (`node_count + 1` little-endian words) |
//! | `CSR_TARGETS_*` | CSR targets array (one little-endian word per edge) |
//!
//! The `_U16` / `_U32` / `_U64` suffix selects the little-endian word width.
//! All section-kind constants are `perf: unspecified` — compile-time `u32`
//! tags.
#![no_std]

#[cfg(feature = "build")]
extern crate alloc;

#[cfg(kani)]
extern crate kani;

#[cfg(kani)]
mod proofs;

#[cfg(feature = "build")]
pub mod build;

use core::{fmt, marker::PhantomData};

use oxgraph_graph::{
    ContainsElement, ContainsRelation, EdgeTargetGraph, ElementIndex, ElementSuccessors,
    GraphCounts, OutgoingEdgeCount, OutgoingGraph, RelationIndex, TopologyBase, TopologyCounts,
};
use oxgraph_layout_util::{
    IdSlice, LocalId, NodeAxis, OffsetIntegrityIssue, SnapshotWidth, check_offset_section,
    check_value_range,
};
pub use oxgraph_layout_util::{
    LayoutIndex as CsrIndex, LayoutSnapshotWord as CsrSnapshotWord, LayoutWord as CsrWord,
};
use oxgraph_snapshot::{SectionBindError, SectionViewError, Snapshot};

/// Section kind for a CSR `u16` offsets array.
pub const SNAPSHOT_KIND_CSR_OFFSETS_U16: u32 = 0x0001;
/// Section kind for a CSR `u32` offsets array.
pub const SNAPSHOT_KIND_CSR_OFFSETS_U32: u32 = 0x0002;
/// Section kind for a CSR `u64` offsets array.
pub const SNAPSHOT_KIND_CSR_OFFSETS_U64: u32 = 0x0003;

/// Section kind for a CSR `u16` targets array.
pub const SNAPSHOT_KIND_CSR_TARGETS_U16: u32 = 0x0004;
/// Section kind for a CSR `u32` targets array.
pub const SNAPSHOT_KIND_CSR_TARGETS_U32: u32 = 0x0005;
/// Section kind for a CSR `u64` targets array.
pub const SNAPSHOT_KIND_CSR_TARGETS_U64: u32 = 0x0006;

/// Section version written and expected for CSR offsets/targets payloads.
pub const SNAPSHOT_CSR_SECTION_VERSION: u32 = 1;

/// Width-specific section-kind tags for persisted CSR offsets/targets payloads.
///
/// This is the thin CSR-specific layer over the shared
/// [`SnapshotWidth`](oxgraph_layout_util::SnapshotWidth) contract: it adds only
/// the per-width section-kind and version constants. The little-endian storage
/// word and the native/LE conversions come from `SnapshotWidth`, so
/// `EdgeIndex::LittleEndianWord` and `EdgeIndex::to_le_word` keep resolving
/// through that trait.
///
/// `usize` deliberately does not implement this trait: snapshot bytes are
/// fixed-width.
///
/// # Performance
///
/// Reading the kind/version constants is `O(1)`.
pub trait CsrSnapshotIndex: SnapshotWidth {
    /// Width-specific CSR offsets section kind.
    ///
    /// # Performance
    ///
    /// Reading this constant is `O(1)`.
    const OFFSETS_KIND: u32;

    /// Width-specific CSR targets section kind.
    ///
    /// # Performance
    ///
    /// Reading this constant is `O(1)`.
    const TARGETS_KIND: u32;

    /// Section version written for this width's CSR payloads.
    ///
    /// # Performance
    ///
    /// Reading this constant is `O(1)`.
    const SECTION_VERSION: u32;
}

/// Implements [`CsrSnapshotIndex`] for one portable snapshot width.
macro_rules! impl_csr_snapshot_index {
    ($index:ty, $offsets_kind:expr, $targets_kind:expr) => {
        impl CsrSnapshotIndex for $index {
            const OFFSETS_KIND: u32 = $offsets_kind;
            const TARGETS_KIND: u32 = $targets_kind;
            const SECTION_VERSION: u32 = SNAPSHOT_CSR_SECTION_VERSION;
        }
    };
}

impl_csr_snapshot_index!(
    u16,
    SNAPSHOT_KIND_CSR_OFFSETS_U16,
    SNAPSHOT_KIND_CSR_TARGETS_U16
);
impl_csr_snapshot_index!(
    u32,
    SNAPSHOT_KIND_CSR_OFFSETS_U32,
    SNAPSHOT_KIND_CSR_TARGETS_U32
);
impl_csr_snapshot_index!(
    u64,
    SNAPSHOT_KIND_CSR_OFFSETS_U64,
    SNAPSHOT_KIND_CSR_TARGETS_U64
);

/// Native borrowed CSR graph alias.
///
/// The node and edge index parameters are spelled explicitly. Target entries
/// use `NodeIndex`, and offset entries use `EdgeIndex`.
///
/// # Performance
///
/// `perf: unspecified`; this alias carries no runtime cost.
pub type CsrNativeGraph<'view, NodeIndex, EdgeIndex> =
    CsrGraph<'view, NodeIndex, EdgeIndex, EdgeIndex, NodeIndex>;

/// Snapshot-backed little-endian CSR graph alias.
///
/// `NodeIndex` selects the target-entry wire width, and `EdgeIndex` selects the
/// offset-entry wire width. Both widths must be portable snapshot widths
/// (`u16`, `u32`, or `u64`).
///
/// # Performance
///
/// `perf: unspecified`; this alias carries no runtime cost.
pub type CsrSnapshotGraph<'view, NodeIndex, EdgeIndex> = CsrGraph<
    'view,
    NodeIndex,
    EdgeIndex,
    <EdgeIndex as SnapshotWidth>::LittleEndianWord,
    <NodeIndex as SnapshotWidth>::LittleEndianWord,
>;

/// Local node ID for [`CsrGraph`].
///
/// Values are dense handles in `0..node_count` for one validated CSR view. They
/// are topology-local IDs and are not stable across rebuilding or compaction
/// unless a higher layer defines that contract. This is an alias of the shared
/// [`LocalId`](oxgraph_layout_util::LocalId) branded by the node axis, so a
/// built graph and its borrowed snapshot view yield the same handle type.
///
/// # Performance
///
/// Copying, comparing, ordering, hashing, and debug-formatting are `O(1)` when
/// the underlying index type provides those operations in `O(1)`.
pub type CsrNodeId<Index> = LocalId<NodeAxis, Index>;

/// Local edge ID for [`CsrGraph`].
///
/// Values are dense handles into the flat CSR target array. They are
/// topology-local IDs and are not stable across sorting, rebuilding, or
/// compaction unless a higher layer defines that contract. This is an alias of
/// the shared [`LocalId`](oxgraph_layout_util::LocalId) branded by the edge
/// axis.
///
/// # Performance
///
/// Copying, comparing, ordering, hashing, and debug-formatting are `O(1)` when
/// the underlying index type provides those operations in `O(1)`.
pub type CsrEdgeId<Index> = LocalId<oxgraph_layout_util::EdgeAxis, Index>;

/// Typestate marker for a slot that still carries an unchecked raw ID.
#[derive(Clone, Copy, Debug)]
struct Unchecked;

/// Typestate marker for a slot checked against a validated CSR view.
#[derive(Clone, Copy, Debug)]
struct Checked;

/// Node slot branded by checked/unchecked typestate.
#[derive(Clone, Copy, Debug)]
struct NodeSlot<State, Index> {
    /// Raw node index value supplied by a public ID.
    raw: Index,
    /// Dense `usize` node slot; meaningful only in the [`Checked`] state.
    slot: usize,
    /// Marker carrying the slot typestate.
    state: PhantomData<fn() -> State>,
}

impl<Index> NodeSlot<Unchecked, Index> {
    /// Creates an unchecked node slot from a public node ID.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    fn from_id(id: CsrNodeId<Index>) -> Self
    where
        Index: Copy,
    {
        Self {
            raw: id.get(),
            slot: 0,
            state: PhantomData,
        }
    }
}

impl<Index> NodeSlot<Checked, Index> {
    /// Creates a checked node slot after graph validation has succeeded.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    fn from_raw_slot(raw: Index, slot: usize) -> Self {
        Self {
            raw,
            slot,
            state: PhantomData,
        }
    }

    /// Returns the dense `usize` node slot.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    const fn index(&self) -> usize {
        self.slot
    }
}

/// Edge slot branded by checked/unchecked typestate.
#[derive(Clone, Copy, Debug)]
struct EdgeSlot<State, Index> {
    /// Raw edge index value supplied by a public ID or reconstructed from a slot.
    raw: Index,
    /// Dense `usize` edge slot; meaningful only in the [`Checked`] state.
    slot: usize,
    /// Marker carrying the slot typestate.
    state: PhantomData<fn() -> State>,
}

impl<Index> EdgeSlot<Unchecked, Index> {
    /// Creates an unchecked edge slot from a public edge ID.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    fn from_id(id: CsrEdgeId<Index>) -> Self
    where
        Index: Copy,
    {
        Self {
            raw: id.get(),
            slot: 0,
            state: PhantomData,
        }
    }
}

impl<Index> EdgeSlot<Checked, Index> {
    /// Creates a checked edge slot after graph validation has succeeded.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    fn from_raw_slot(raw: Index, slot: usize) -> Self {
        Self {
            raw,
            slot,
            state: PhantomData,
        }
    }

    /// Reconstructs a checked edge slot from a validated CSR range position.
    ///
    /// # Panics
    ///
    /// Panics via `unreachable!()` only if CSR validation or range construction
    /// has been bypassed inside this module. Public callers cannot construct a
    /// checked edge range.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    fn from_csr_range_slot(slot: usize) -> Option<Self>
    where
        Index: CsrIndex,
    {
        let raw = oxgraph_layout_util::usize_to_index_validated::<Index>(slot)?;
        Some(Self::from_raw_slot(raw, slot))
    }

    /// Reconstructs a checked edge slot from a validated CSR range position.
    ///
    /// # Panics
    ///
    /// Panics via `unreachable!()` only if CSR validation or range construction
    /// has been bypassed inside this module. Public callers cannot construct a
    /// checked edge range.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    fn from_csr_range_slot_unchecked(slot: usize) -> Self
    where
        Index: CsrIndex,
    {
        Self::from_csr_range_slot(slot)
            .unwrap_or_else(|| unreachable!("checked CSR edge slot must fit index type"))
    }

    /// Returns the dense `usize` edge slot.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    const fn index(&self) -> usize {
        self.slot
    }

    /// Returns this checked edge slot as a public edge ID.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    const fn id(&self) -> CsrEdgeId<Index>
    where
        Index: Copy,
    {
        CsrEdgeId::new(self.raw)
    }
}

/// Edge-slot range branded by checked/unchecked typestate.
#[derive(Clone, Copy, Debug)]
struct EdgeRange<State, Index> {
    /// Inclusive start slot.
    start: usize,
    /// Exclusive end slot.
    end: usize,
    /// Marker carrying the range typestate.
    state: PhantomData<fn() -> State>,
    /// Marker carrying the logical index type.
    index: PhantomData<fn() -> Index>,
}

impl<Index> EdgeRange<Checked, Index>
where
    Index: CsrIndex,
{
    /// Creates a checked edge range after CSR row offsets have been validated.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    fn from_bounds(start: usize, end: usize) -> Self {
        Self {
            start,
            end,
            state: PhantomData,
            index: PhantomData,
        }
    }

    /// Returns this range as a standard `usize` range.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    const fn as_range(&self) -> core::ops::Range<usize> {
        self.start..self.end
    }

    /// Returns the number of slots remaining in this range.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    const fn len(&self) -> usize {
        self.end - self.start
    }

    /// Advances the range and returns the next checked edge slot.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    fn next_slot(&mut self) -> Option<EdgeSlot<Checked, Index>> {
        if self.start == self.end {
            return None;
        }

        let slot = EdgeSlot::from_csr_range_slot_unchecked(self.start);
        self.start += 1;
        Some(slot)
    }
}

/// Borrowed compressed-sparse-row graph view.
///
/// The graph stores outgoing adjacency using `offsets[node]..offsets[node + 1]`
/// ranges into the flat `targets` slice. The view borrows both slices and does
/// not allocate. `NodeIndex` is the host-endian logical index type used for
/// node IDs and target entries. `EdgeIndex` is the host-endian logical index
/// type used for edge IDs and offset entries. The borrowed offset and target
/// slices may use native words or matching little-endian zerocopy words.
///
/// # Performance
///
/// Creating a validated view is `O(n + m)` for `n` nodes and `m` edges because
/// validation checks monotonic offsets and target bounds. Outgoing traversal for
/// one node is `O(1)` to create and `O(k)` to yield `k` outgoing edges.
#[derive(Clone, Copy, Debug)]
pub struct CsrGraph<'view, NodeIndex, EdgeIndex, OffsetWord, TargetWord>
where
    NodeIndex: CsrIndex,
    EdgeIndex: CsrIndex,
    OffsetWord: CsrWord<Index = EdgeIndex>,
    TargetWord: CsrWord<Index = NodeIndex>,
{
    /// Number of nodes in the graph as the public logical index type.
    node_count: NodeIndex,
    /// Number of nodes cached as a validated `usize` slot bound.
    node_bound: usize,
    /// CSR offsets with length `node_count + 1`.
    offsets: &'view [OffsetWord],
    /// Flat outgoing target node IDs.
    targets: &'view [TargetWord],
}

impl<'view, NodeIndex, EdgeIndex, OffsetWord, TargetWord>
    CsrGraph<'view, NodeIndex, EdgeIndex, OffsetWord, TargetWord>
where
    NodeIndex: CsrIndex,
    EdgeIndex: CsrIndex,
    OffsetWord: CsrWord<Index = EdgeIndex>,
    TargetWord: CsrWord<Index = NodeIndex>,
{
    /// Validates borrowed CSR sections and returns a graph view.
    ///
    /// # Errors
    ///
    /// Returns [`CsrError`] when offsets have the wrong length, offsets are not
    /// monotonic, the final offset does not match `targets.len()`, a target is
    /// out of range, or an index count cannot be represented as `usize` on the
    /// current target.
    ///
    /// # Performance
    ///
    /// Validation is `O(n + m)` for `n` nodes and `m` edges.
    pub fn validate(
        node_count: NodeIndex,
        offsets: &'view [OffsetWord],
        targets: &'view [TargetWord],
    ) -> Result<Self, CsrError<NodeIndex, EdgeIndex>> {
        let node_bound = node_count
            .to_usize()
            .ok_or(CsrError::NodeUsizeOverflow { value: node_count })?;
        if node_bound.checked_add(1).is_none() {
            return Err(CsrError::OffsetLengthOverflow { node_count });
        }

        check_offset_section(offsets, node_bound, targets.len())
            .map_err(|issue| map_offsets_issue::<NodeIndex, EdgeIndex, _>(offsets, issue))?;
        check_value_range(targets, node_bound).map_err(|issue| {
            map_targets_issue::<NodeIndex, EdgeIndex, _>(targets, node_count, issue)
        })?;

        Ok(Self {
            node_count,
            node_bound,
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
    pub const fn offsets(&self) -> &'view [OffsetWord] {
        self.offsets
    }

    /// Returns the borrowed CSR target slice.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub const fn targets(&self) -> &'view [TargetWord] {
        self.targets
    }

    /// Walks outgoing target node ids for `source` via the CSR target slice.
    ///
    /// Stops early when `visit` returns `true`. Returns `true` when stopped early.
    ///
    /// # Performance
    ///
    /// This method is `O(k)` for `k` outgoing edges with no iterator adapters.
    pub fn for_each_out_target(
        &self,
        source: CsrNodeId<NodeIndex>,
        mut visit: impl FnMut(CsrNodeId<NodeIndex>) -> bool,
    ) -> bool {
        let Some(node) = self.try_node_slot(source) else {
            return false;
        };
        let Some(index) = node.raw.to_usize() else {
            return false;
        };
        let Some(start) = self.offsets[index].get().to_usize() else {
            return false;
        };
        let Some(end) = self.offsets[index + 1].get().to_usize() else {
            return false;
        };
        for word in &self.targets[start..end] {
            if visit(CsrNodeId::new(word.get())) {
                return true;
            }
        }
        false
    }

    /// Returns whether `node` is valid in this CSR view.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub fn contains_node(&self, node: CsrNodeId<NodeIndex>) -> bool {
        self.try_node_slot(node).is_some()
    }

    /// Returns whether `edge` is valid in this CSR view.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub fn contains_edge(&self, edge: CsrEdgeId<EdgeIndex>) -> bool {
        self.try_edge_slot(edge).is_some()
    }

    /// Returns the target node for `edge` when `edge` is valid in this view.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub fn try_target(&self, edge: CsrEdgeId<EdgeIndex>) -> Option<CsrNodeId<NodeIndex>> {
        self.try_edge_slot(edge)
            .map(|checked| self.target_node(checked))
    }

    /// Checks a public node ID and returns a checked node slot on success.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    fn try_node_slot(&self, node: CsrNodeId<NodeIndex>) -> Option<NodeSlot<Checked, NodeIndex>> {
        self.check_node_slot(NodeSlot::from_id(node))
    }

    /// Converts an unchecked node slot into a checked node slot.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    fn check_node_slot(
        &self,
        node: NodeSlot<Unchecked, NodeIndex>,
    ) -> Option<NodeSlot<Checked, NodeIndex>> {
        let slot = node.raw.to_usize()?;
        if node.raw < self.node_count && slot < self.node_bound {
            Some(NodeSlot::from_raw_slot(node.raw, slot))
        } else {
            None
        }
    }

    /// Returns a checked node slot or panics on a topology contract violation.
    ///
    /// # Panics
    ///
    /// Panics when `node` is not a valid node ID for this CSR view. Graph trait
    /// methods that call this helper require valid IDs by contract.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    fn checked_node_slot(&self, node: CsrNodeId<NodeIndex>) -> NodeSlot<Checked, NodeIndex> {
        self.try_node_slot(node)
            .unwrap_or_else(|| panic!("CSR node ID {node:?} is invalid for this graph"))
    }

    /// Checks a public edge ID and returns a checked edge slot on success.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    fn try_edge_slot(&self, edge: CsrEdgeId<EdgeIndex>) -> Option<EdgeSlot<Checked, EdgeIndex>> {
        self.check_edge_slot(EdgeSlot::from_id(edge))
    }

    /// Converts an unchecked edge slot into a checked edge slot.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    fn check_edge_slot(
        &self,
        edge: EdgeSlot<Unchecked, EdgeIndex>,
    ) -> Option<EdgeSlot<Checked, EdgeIndex>> {
        let slot = edge.raw.to_usize()?;
        if slot < self.targets.len() {
            Some(EdgeSlot::from_raw_slot(edge.raw, slot))
        } else {
            None
        }
    }

    /// Returns a checked edge slot or panics on a topology contract violation.
    ///
    /// # Panics
    ///
    /// Panics when `edge` is not a valid edge ID for this CSR view. Graph trait
    /// methods that call this helper require valid IDs by contract.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    fn checked_edge_slot(&self, edge: CsrEdgeId<EdgeIndex>) -> EdgeSlot<Checked, EdgeIndex> {
        self.try_edge_slot(edge)
            .unwrap_or_else(|| panic!("CSR edge ID {edge:?} is invalid for this graph"))
    }

    /// Converts a CSR offset value from a validated row into a `usize` slot.
    ///
    /// # Panics
    ///
    /// Panics via `unreachable!()` only if CSR validation has been bypassed
    /// inside this module. Validation checks that the final offset fits in
    /// `usize`, and monotonicity ensures every row offset is at most that final
    /// offset.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    fn checked_offset_slot(offset: EdgeIndex) -> usize {
        oxgraph_layout_util::index_to_usize_validated(offset)
            .unwrap_or_else(|| unreachable!("checked CSR offset must fit usize"))
    }

    /// Returns the target node for a checked CSR edge slot.
    ///
    /// # Performance
    ///
    /// This method is `O(1)` for valid edge IDs from this view.
    fn target_node(&self, edge: EdgeSlot<Checked, EdgeIndex>) -> CsrNodeId<NodeIndex> {
        CsrNodeId::new(self.targets[edge.index()].get())
    }

    /// Returns the start and end edge slots for a checked node.
    ///
    /// # Performance
    ///
    /// This method is `O(1)` for valid node IDs from this view.
    fn outgoing_range(&self, node: NodeSlot<Checked, NodeIndex>) -> EdgeRange<Checked, EdgeIndex> {
        let index = node.index();
        EdgeRange::from_bounds(
            Self::checked_offset_slot(self.offsets[index].get()),
            Self::checked_offset_slot(self.offsets[index + 1].get()),
        )
    }
}

impl<'view, NodeIndex, EdgeIndex>
    CsrGraph<
        'view,
        NodeIndex,
        EdgeIndex,
        <EdgeIndex as SnapshotWidth>::LittleEndianWord,
        <NodeIndex as SnapshotWidth>::LittleEndianWord,
    >
where
    NodeIndex: CsrSnapshotIndex,
    EdgeIndex: CsrSnapshotIndex,
{
    /// Builds a snapshot-backed CSR view from a validated [`Snapshot`].
    ///
    /// Reads the width-specific CSR offsets and targets sections, borrows them
    /// as little-endian index words, derives `node_count` from
    /// `offsets.len() - 1`, and runs CSR-shape validation. The returned view
    /// borrows directly from the snapshot's byte slice and does not copy. Use
    /// [`CsrSnapshotGraph`] to select node and edge snapshot widths, for example
    /// `CsrSnapshotGraph<'_, u32, u64>`.
    ///
    /// This delegates to [`from_snapshot_with_kinds`](Self::from_snapshot_with_kinds)
    /// using the width-default offsets/targets kinds and section version.
    ///
    /// # Errors
    ///
    /// Returns [`CsrSnapshotError`] when either section is missing, has the
    /// wrong version, cannot be viewed as the selected word width, is empty, has
    /// too many offsets for the selected index type, or fails CSR validation.
    ///
    /// # Performance
    ///
    /// This function is `O(s + n + m)` for `s` snapshot sections, `n` graph
    /// nodes, and `m` graph edges.
    pub fn from_snapshot(
        snapshot: &Snapshot<'view>,
    ) -> Result<Self, CsrSnapshotError<NodeIndex, EdgeIndex>> {
        Self::from_snapshot_with_kinds(
            snapshot,
            EdgeIndex::OFFSETS_KIND,
            NodeIndex::TARGETS_KIND,
            EdgeIndex::SECTION_VERSION,
        )
    }

    /// Builds a snapshot-backed CSR view using caller-chosen section kinds.
    ///
    /// Identical to [`from_snapshot`](Self::from_snapshot) but with the offsets
    /// and targets section kinds and the section version supplied explicitly, so
    /// storage layers that persist CSR in a non-default band (for example an
    /// inbound CSC index) can reuse the same validated borrow logic.
    ///
    /// # Errors
    ///
    /// Returns [`CsrSnapshotError`] when either section is missing, has the
    /// wrong version, cannot be viewed as the selected word width, is empty, has
    /// too many offsets for the selected index type, or fails CSR validation.
    ///
    /// # Performance
    ///
    /// This function is `O(s + n + m)` for `s` snapshot sections, `n` graph
    /// nodes, and `m` graph edges.
    pub fn from_snapshot_with_kinds(
        snapshot: &Snapshot<'view>,
        offsets_kind: u32,
        targets_kind: u32,
        version: u32,
    ) -> Result<Self, CsrSnapshotError<NodeIndex, EdgeIndex>> {
        let offsets = snapshot
            .typed_section::<EdgeIndex>(offsets_kind, version)
            .map_err(|error| map_offsets_bind(offsets_kind, error))?;
        let targets = snapshot
            .typed_section::<NodeIndex>(targets_kind, version)
            .map_err(|error| map_targets_bind(targets_kind, error))?;

        if offsets.is_empty() {
            return Err(CsrSnapshotError::OffsetsEmpty);
        }

        let node_count_usize = offsets.len() - 1;
        let node_count =
            NodeIndex::from_usize(node_count_usize).ok_or(CsrSnapshotError::NodeCountOverflow {
                offsets_len: offsets.len(),
            })?;

        Ok(Self::validate(node_count, offsets, targets)?)
    }
}

/// Maps an offsets-side [`SectionBindError`] into a typed [`CsrSnapshotError`].
const fn map_offsets_bind<NodeIndex, EdgeIndex>(
    kind: u32,
    error: SectionBindError,
) -> CsrSnapshotError<NodeIndex, EdgeIndex> {
    match error {
        SectionBindError::Missing { .. } => CsrSnapshotError::MissingOffsets,
        SectionBindError::VersionMismatch {
            expected, actual, ..
        } => CsrSnapshotError::OffsetsVersion {
            kind,
            expected,
            actual,
        },
        SectionBindError::View { error, .. } => CsrSnapshotError::OffsetsView(error),
    }
}

/// Maps a targets-side [`SectionBindError`] into a typed [`CsrSnapshotError`].
const fn map_targets_bind<NodeIndex, EdgeIndex>(
    kind: u32,
    error: SectionBindError,
) -> CsrSnapshotError<NodeIndex, EdgeIndex> {
    match error {
        SectionBindError::Missing { .. } => CsrSnapshotError::MissingTargets,
        SectionBindError::VersionMismatch {
            expected, actual, ..
        } => CsrSnapshotError::TargetsVersion {
            kind,
            expected,
            actual,
        },
        SectionBindError::View { error, .. } => CsrSnapshotError::TargetsView(error),
    }
}

impl<NodeIndex, EdgeIndex, OffsetWord, TargetWord> TopologyBase
    for CsrGraph<'_, NodeIndex, EdgeIndex, OffsetWord, TargetWord>
where
    NodeIndex: CsrIndex,
    EdgeIndex: CsrIndex,
    OffsetWord: CsrWord<Index = EdgeIndex>,
    TargetWord: CsrWord<Index = NodeIndex>,
{
    type ElementId = CsrNodeId<NodeIndex>;
    type RelationId = CsrEdgeId<EdgeIndex>;
}

impl<NodeIndex, EdgeIndex, OffsetWord, TargetWord> TopologyCounts
    for CsrGraph<'_, NodeIndex, EdgeIndex, OffsetWord, TargetWord>
where
    NodeIndex: CsrIndex,
    EdgeIndex: CsrIndex,
    OffsetWord: CsrWord<Index = EdgeIndex>,
    TargetWord: CsrWord<Index = NodeIndex>,
{
    fn element_count(&self) -> usize {
        self.node_bound
    }

    fn relation_count(&self) -> usize {
        self.targets.len()
    }
}

impl<NodeIndex, EdgeIndex, OffsetWord, TargetWord> GraphCounts
    for CsrGraph<'_, NodeIndex, EdgeIndex, OffsetWord, TargetWord>
where
    NodeIndex: CsrIndex,
    EdgeIndex: CsrIndex,
    OffsetWord: CsrWord<Index = EdgeIndex>,
    TargetWord: CsrWord<Index = NodeIndex>,
{
}

impl<NodeIndex, EdgeIndex, OffsetWord, TargetWord> ElementIndex
    for CsrGraph<'_, NodeIndex, EdgeIndex, OffsetWord, TargetWord>
where
    NodeIndex: CsrIndex,
    EdgeIndex: CsrIndex,
    OffsetWord: CsrWord<Index = EdgeIndex>,
    TargetWord: CsrWord<Index = NodeIndex>,
{
    fn element_bound(&self) -> usize {
        self.node_bound
    }

    fn element_index(&self, element: CsrNodeId<NodeIndex>) -> usize {
        self.checked_node_slot(element).index()
    }
}

impl<NodeIndex, EdgeIndex, OffsetWord, TargetWord> RelationIndex
    for CsrGraph<'_, NodeIndex, EdgeIndex, OffsetWord, TargetWord>
where
    NodeIndex: CsrIndex,
    EdgeIndex: CsrIndex,
    OffsetWord: CsrWord<Index = EdgeIndex>,
    TargetWord: CsrWord<Index = NodeIndex>,
{
    fn relation_bound(&self) -> usize {
        self.targets.len()
    }

    fn relation_index(&self, relation: CsrEdgeId<EdgeIndex>) -> usize {
        self.checked_edge_slot(relation).index()
    }
}

impl<NodeIndex, EdgeIndex, OffsetWord, TargetWord> ContainsElement
    for CsrGraph<'_, NodeIndex, EdgeIndex, OffsetWord, TargetWord>
where
    NodeIndex: CsrIndex,
    EdgeIndex: CsrIndex,
    OffsetWord: CsrWord<Index = EdgeIndex>,
    TargetWord: CsrWord<Index = NodeIndex>,
{
    fn contains_element(&self, element: CsrNodeId<NodeIndex>) -> bool {
        self.contains_node(element)
    }
}

impl<NodeIndex, EdgeIndex, OffsetWord, TargetWord> ContainsRelation
    for CsrGraph<'_, NodeIndex, EdgeIndex, OffsetWord, TargetWord>
where
    NodeIndex: CsrIndex,
    EdgeIndex: CsrIndex,
    OffsetWord: CsrWord<Index = EdgeIndex>,
    TargetWord: CsrWord<Index = NodeIndex>,
{
    fn contains_relation(&self, relation: CsrEdgeId<EdgeIndex>) -> bool {
        self.contains_edge(relation)
    }
}

impl<NodeIndex, EdgeIndex, OffsetWord, TargetWord> EdgeTargetGraph
    for CsrGraph<'_, NodeIndex, EdgeIndex, OffsetWord, TargetWord>
where
    NodeIndex: CsrIndex,
    EdgeIndex: CsrIndex,
    OffsetWord: CsrWord<Index = EdgeIndex>,
    TargetWord: CsrWord<Index = NodeIndex>,
{
    fn target(&self, edge: CsrEdgeId<EdgeIndex>) -> CsrNodeId<NodeIndex> {
        self.target_node(self.checked_edge_slot(edge))
    }
}

impl<NodeIndex, EdgeIndex, OffsetWord, TargetWord> OutgoingGraph
    for CsrGraph<'_, NodeIndex, EdgeIndex, OffsetWord, TargetWord>
where
    NodeIndex: CsrIndex,
    EdgeIndex: CsrIndex,
    OffsetWord: CsrWord<Index = EdgeIndex>,
    TargetWord: CsrWord<Index = NodeIndex>,
{
    type OutEdges<'view>
        = CsrOutEdges<EdgeIndex>
    where
        Self: 'view;

    fn outgoing_edges(&self, node: CsrNodeId<NodeIndex>) -> Self::OutEdges<'_> {
        CsrOutEdges {
            range: self.outgoing_range(self.checked_node_slot(node)),
        }
    }
}

impl<NodeIndex, EdgeIndex, OffsetWord, TargetWord> OutgoingEdgeCount
    for CsrGraph<'_, NodeIndex, EdgeIndex, OffsetWord, TargetWord>
where
    NodeIndex: CsrIndex,
    EdgeIndex: CsrIndex,
    OffsetWord: CsrWord<Index = EdgeIndex>,
    TargetWord: CsrWord<Index = NodeIndex>,
{
    fn out_degree(&self, node: CsrNodeId<NodeIndex>) -> usize {
        self.outgoing_range(self.checked_node_slot(node)).len()
    }
}

impl<NodeIndex, EdgeIndex, OffsetWord, TargetWord> ElementSuccessors
    for CsrGraph<'_, NodeIndex, EdgeIndex, OffsetWord, TargetWord>
where
    NodeIndex: CsrIndex,
    EdgeIndex: CsrIndex,
    OffsetWord: CsrWord<Index = EdgeIndex>,
    TargetWord: CsrWord<Index = NodeIndex>,
{
    type Successors<'view>
        = IdSlice<'view, TargetWord, CsrNodeId<NodeIndex>>
    where
        Self: 'view;

    fn element_successors(&self, node: CsrNodeId<NodeIndex>) -> Self::Successors<'_> {
        let range = self.outgoing_range(self.checked_node_slot(node));
        IdSlice::new(&self.targets[range.as_range()])
    }
}

/// Iterator over outgoing CSR edge slots.
///
/// # Performance
///
/// Advancing the iterator is `O(1)`.
#[derive(Clone, Debug)]
pub struct CsrOutEdges<Index> {
    /// Checked outgoing edge range remaining to yield.
    range: EdgeRange<Checked, Index>,
}

impl<Index> Iterator for CsrOutEdges<Index>
where
    Index: CsrIndex,
{
    type Item = CsrEdgeId<Index>;

    fn next(&mut self) -> Option<Self::Item> {
        self.range.next_slot().map(|slot| slot.id())
    }
}

impl<Index> ExactSizeIterator for CsrOutEdges<Index>
where
    Index: CsrIndex,
{
    fn len(&self) -> usize {
        self.range.len()
    }
}

/// Maps an offset-side [`OffsetIntegrityIssue`] into a typed [`CsrError`],
/// reading the offending word out of `offsets` to populate typed fields.
///
/// `from_usize` fallbacks would zero-out diagnostic state, so we recover the
/// typed offset by indexing back into the original word slice — that read is
/// what produced the issue in the first place and is guaranteed in-bounds.
fn map_offsets_issue<NodeIndex, EdgeIndex, OffsetWord>(
    offsets: &[OffsetWord],
    issue: OffsetIntegrityIssue,
) -> CsrError<NodeIndex, EdgeIndex>
where
    NodeIndex: CsrIndex,
    EdgeIndex: CsrIndex,
    OffsetWord: CsrWord<Index = EdgeIndex>,
{
    match issue {
        OffsetIntegrityIssue::Length { expected, actual } => {
            CsrError::OffsetLength { expected, actual }
        }
        OffsetIntegrityIssue::FirstNonZero { .. } => CsrError::FirstOffset {
            actual: offsets[0].get(),
        },
        OffsetIntegrityIssue::NonMonotonic { index, .. } => CsrError::NonMonotonicOffset {
            index,
            previous: offsets[index - 1].get(),
            actual: offsets[index].get(),
        },
        OffsetIntegrityIssue::FinalMismatch { value_len, .. } => CsrError::FinalOffset {
            final_offset: offsets[offsets.len() - 1].get(),
            target_len: value_len,
        },
        OffsetIntegrityIssue::UsizeOverflow { index } => CsrError::EdgeUsizeOverflow {
            value: offsets[index].get(),
        },
        OffsetIntegrityIssue::ValueOutOfRange { .. } => {
            // Offset section never produces ValueOutOfRange; treat as overflow.
            CsrError::EdgeUsizeOverflow {
                value: EdgeIndex::ZERO,
            }
        }
        _ => CsrError::EdgeUsizeOverflow {
            value: EdgeIndex::ZERO,
        },
    }
}

/// Maps a target-side [`OffsetIntegrityIssue`] into a typed [`CsrError`],
/// reading the offending word out of `targets` and preserving the typed
/// `node_count` bound the caller supplied to `check_value_range`.
fn map_targets_issue<NodeIndex, EdgeIndex, TargetWord>(
    targets: &[TargetWord],
    node_count: NodeIndex,
    issue: OffsetIntegrityIssue,
) -> CsrError<NodeIndex, EdgeIndex>
where
    NodeIndex: CsrIndex,
    EdgeIndex: CsrIndex,
    TargetWord: CsrWord<Index = NodeIndex>,
{
    match issue {
        OffsetIntegrityIssue::ValueOutOfRange { index, .. } => CsrError::TargetOutOfRange {
            index,
            target: targets[index].get(),
            node_count,
        },
        // A target word that does not fit `usize` (a 32-bit-host width failure)
        // is distinct from a target at/above `node_count`; report it as such
        // rather than conflating it with an out-of-range target.
        OffsetIntegrityIssue::UsizeOverflow { index } => CsrError::TargetUsizeOverflow {
            index,
            value: targets[index].get(),
        },
        _ => CsrError::TargetOutOfRange {
            index: 0,
            target: NodeIndex::ZERO,
            node_count,
        },
    }
}

/// CSR validation error.
///
/// # Performance
///
/// `perf: unspecified`; errors are returned only from validation paths.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CsrError<NodeIndex, EdgeIndex> {
    /// `node_count + 1` overflowed `usize`.
    OffsetLengthOverflow {
        /// Node count that could not be converted to an offset length.
        node_count: NodeIndex,
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
        actual: EdgeIndex,
    },
    /// Offsets were not monotonically increasing.
    NonMonotonicOffset {
        /// Offset index where monotonicity failed.
        index: usize,
        /// Previous offset value.
        previous: EdgeIndex,
        /// Actual offset value at `index`.
        actual: EdgeIndex,
    },
    /// Final offset does not match target slice length.
    FinalOffset {
        /// Final offset value.
        final_offset: EdgeIndex,
        /// Target slice length.
        target_len: usize,
    },
    /// Target node ID is outside `0..node_count`.
    TargetOutOfRange {
        /// Target slice index containing the bad value.
        index: usize,
        /// Bad target node ID.
        target: NodeIndex,
        /// Number of nodes in the graph.
        node_count: NodeIndex,
    },
    /// A target node ID value could not be represented as `usize` on this
    /// target (a width failure, distinct from an out-of-range target).
    TargetUsizeOverflow {
        /// Target slice index containing the value.
        index: usize,
        /// Target value that did not fit in `usize`.
        value: NodeIndex,
    },
    /// A node index value could not be represented as `usize` on this target.
    NodeUsizeOverflow {
        /// Node value that could not be represented as `usize`.
        value: NodeIndex,
    },
    /// An edge index value could not be represented as `usize` on this target.
    EdgeUsizeOverflow {
        /// Edge value that could not be represented as `usize`.
        value: EdgeIndex,
    },
}

impl<NodeIndex, EdgeIndex> fmt::Display for CsrError<NodeIndex, EdgeIndex>
where
    NodeIndex: fmt::Display,
    EdgeIndex: fmt::Display,
{
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
            Self::TargetUsizeOverflow { index, value } => write!(
                formatter,
                "CSR target at index {index} value {value} does not fit usize"
            ),
            Self::NodeUsizeOverflow { value } => {
                write!(formatter, "CSR node index value {value} does not fit usize")
            }
            Self::EdgeUsizeOverflow { value } => {
                write!(formatter, "CSR edge index value {value} does not fit usize")
            }
        }
    }
}

impl<NodeIndex, EdgeIndex> core::error::Error for CsrError<NodeIndex, EdgeIndex>
where
    NodeIndex: fmt::Debug + fmt::Display,
    EdgeIndex: fmt::Debug + fmt::Display,
{
}

/// Error returned when a snapshot cannot be opened as a CSR graph.
///
/// # Performance
///
/// `perf: unspecified`; errors are returned only from snapshot-bound paths.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CsrSnapshotError<NodeIndex, EdgeIndex> {
    /// The snapshot has no CSR offsets section for the requested edge width.
    MissingOffsets,
    /// The snapshot has no CSR targets section for the requested node width.
    MissingTargets,
    /// The CSR offsets section was present but its version did not match.
    OffsetsVersion {
        /// Offsets section kind that was bound.
        kind: u32,
        /// Section version the reader required.
        expected: u32,
        /// Section version recorded in the snapshot.
        actual: u32,
    },
    /// The CSR targets section was present but its version did not match.
    TargetsVersion {
        /// Targets section kind that was bound.
        kind: u32,
        /// Section version the reader required.
        expected: u32,
        /// Section version recorded in the snapshot.
        actual: u32,
    },
    /// The CSR offsets section payload could not be borrowed as the selected
    /// little-endian index word slice.
    OffsetsView(SectionViewError),
    /// The CSR targets section payload could not be borrowed as the selected
    /// little-endian index word slice.
    TargetsView(SectionViewError),
    /// The CSR offsets section is empty; CSR requires at least one entry for
    /// the n-plus-one layout.
    OffsetsEmpty,
    /// The derived node count would not fit in the selected index type.
    NodeCountOverflow {
        /// Length of the offsets section.
        offsets_len: usize,
    },
    /// CSR-shape validation failed on the borrowed sections.
    Csr(CsrError<NodeIndex, EdgeIndex>),
}

impl<NodeIndex, EdgeIndex> fmt::Display for CsrSnapshotError<NodeIndex, EdgeIndex>
where
    NodeIndex: fmt::Display,
    EdgeIndex: fmt::Display,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingOffsets => formatter.write_str("snapshot has no CSR offsets section"),
            Self::MissingTargets => formatter.write_str("snapshot has no CSR targets section"),
            Self::OffsetsVersion {
                kind,
                expected,
                actual,
            } => write!(
                formatter,
                "CSR offsets section {kind} version {actual} does not match expected {expected}"
            ),
            Self::TargetsVersion {
                kind,
                expected,
                actual,
            } => write!(
                formatter,
                "CSR targets section {kind} version {actual} does not match expected {expected}"
            ),
            Self::OffsetsView(error) => write!(
                formatter,
                "CSR offsets section cannot be borrowed as selected little-endian index words: {error}"
            ),
            Self::TargetsView(error) => write!(
                formatter,
                "CSR targets section cannot be borrowed as selected little-endian index words: {error}"
            ),
            Self::OffsetsEmpty => formatter.write_str("CSR offsets section is empty"),
            Self::NodeCountOverflow { offsets_len } => write!(
                formatter,
                "derived node count from offsets length {offsets_len} does not fit selected CSR index type"
            ),
            Self::Csr(error) => write!(formatter, "CSR validation failed: {error}"),
        }
    }
}

impl<NodeIndex, EdgeIndex> core::error::Error for CsrSnapshotError<NodeIndex, EdgeIndex>
where
    NodeIndex: fmt::Debug + fmt::Display,
    EdgeIndex: fmt::Debug + fmt::Display,
{
}

impl<NodeIndex, EdgeIndex> From<CsrError<NodeIndex, EdgeIndex>>
    for CsrSnapshotError<NodeIndex, EdgeIndex>
{
    fn from(error: CsrError<NodeIndex, EdgeIndex>) -> Self {
        Self::Csr(error)
    }
}
