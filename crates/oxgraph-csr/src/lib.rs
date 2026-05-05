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

use core::{fmt, hash::Hash, marker::PhantomData};

use oxgraph_graph::{
    ContainsElement, ContainsRelation, EdgeTargetGraph, ElementIndex, ElementSuccessors,
    GraphCounts, OutgoingEdgeCount, OutgoingGraph, RelationIndex, TopologyBase, TopologyCounts,
};
use oxgraph_snapshot::{SectionViewError, Snapshot};
use zerocopy::{
    FromBytes, Immutable, KnownLayout,
    byteorder::{LE, U16, U32, U64, Usize},
};

/// Section kind for a CSR offsets array stored in an `oxgraph-snapshot`.
///
/// The payload is a sequence of unaligned little-endian index words of length
/// `node_count + 1`. The concrete word width is selected by the consumer's
/// [`CsrSnapshotGraph`] index type; the CSR view derives `node_count` from this
/// length, so no separate metadata section is required.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub const SNAPSHOT_KIND_CSR_OFFSETS: u32 = 0x0001;

/// Section kind for a CSR targets array stored in an `oxgraph-snapshot`.
///
/// The payload is a sequence of unaligned little-endian index words whose
/// length equals the final value of the CSR offsets array. The concrete word
/// width is selected by the consumer's [`CsrSnapshotGraph`] index type.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub const SNAPSHOT_KIND_CSR_TARGETS: u32 = 0x0002;

/// Private sealing traits for CSR-supported index and snapshot word types.
mod sealed {
    /// Seals [`super::CsrIndex`] to the built-in unsigned integer widths.
    pub trait CsrIndex {}

    /// Seals [`super::CsrSnapshotWord`] to built-in little-endian storage words.
    pub trait CsrSnapshotWord {}
}

/// Unsigned native index type usable as a CSR node, edge, offset, and target.
///
/// CSR uses one logical index type per view. Implementations are the supported
/// unsigned widths (`u16`, `u32`, `u64`, and `usize`) and expose checked
/// conversions to and from `usize`, because Rust slices are indexed by `usize`.
///
/// Raw indexes deliberately expose only checked conversions. Infallible
/// conversion is private to CSR internals and requires a checked slot witness.
///
/// ```compile_fail
/// use oxgraph_csr::CsrIndex;
///
/// let raw = 1_u32;
/// let _ = raw.to_usize_validated();
/// let _ = <u32 as CsrIndex>::from_usize_validated(0);
/// ```
///
/// # Performance
///
/// Copying, comparing, hashing, formatting, and converting supported values are
/// expected to be `O(1)`.
pub trait CsrIndex:
    sealed::CsrIndex + Copy + Eq + Ord + fmt::Debug + fmt::Display + Hash + Sized
{
    /// Little-endian zerocopy storage word for this logical index type.
    ///
    /// # Performance
    ///
    /// `perf: unspecified`; this associated type carries no runtime cost.
    type LittleEndianWord: CsrSnapshotWord<Index = Self>;

    /// Zero value for this index type.
    ///
    /// # Performance
    ///
    /// Reading this constant is `O(1)`.
    const ZERO: Self;

    /// Converts this index value to `usize` when representable on this target.
    ///
    /// # Errors
    ///
    /// Returns [`CsrError::UsizeOverflow`] when this index value does not fit
    /// in `usize` on the current target.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    fn to_usize(self) -> Result<usize, CsrError<Self>>;

    /// Converts a `usize` to this index type when representable.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    fn from_usize(value: usize) -> Option<Self>;
}

/// Implements [`CsrIndex`] for one native unsigned integer type.
macro_rules! impl_csr_index {
    ($index:ty, $little_endian:ty) => {
        impl sealed::CsrIndex for $index {}

        impl CsrIndex for $index {
            type LittleEndianWord = $little_endian;

            const ZERO: Self = 0;

            fn to_usize(self) -> Result<usize, CsrError<Self>> {
                usize::try_from(self).map_err(|_error| CsrError::UsizeOverflow { value: self })
            }

            fn from_usize(value: usize) -> Option<Self> {
                Self::try_from(value).ok()
            }
        }
    };
}

impl_csr_index!(u16, U16<LE>);
impl_csr_index!(u32, U32<LE>);
impl_csr_index!(u64, U64<LE>);

impl sealed::CsrIndex for usize {}

impl CsrIndex for usize {
    type LittleEndianWord = Usize<LE>;

    const ZERO: Self = 0;

    fn to_usize(self) -> Result<usize, CsrError<Self>> {
        Ok(self)
    }

    fn from_usize(value: usize) -> Option<Self> {
        Some(value)
    }
}

/// Integer word usable in borrowed CSR sections.
///
/// Native in-memory graph fixtures use the same type as their logical
/// [`CsrIndex`]. Snapshot-backed views use unaligned little-endian zerocopy
/// wrappers such as [`U32<LE>`] and still expose the same host-endian graph ID
/// type through [`Self::Index`].
///
/// # Performance
///
/// Reading a word is expected to be `O(1)`.
pub trait CsrWord: Copy {
    /// Host-endian logical index decoded from this word.
    ///
    /// # Performance
    ///
    /// `perf: unspecified`; this associated type carries no runtime cost.
    type Index: CsrIndex;

    /// Returns this CSR word as a host-endian index.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    fn get(self) -> Self::Index;
}

/// Implements [`CsrWord`] for one native in-memory index word.
macro_rules! impl_native_csr_word {
    ($index:ty) => {
        impl CsrWord for $index {
            type Index = $index;

            fn get(self) -> Self::Index {
                self
            }
        }
    };
}

impl_native_csr_word!(u16);
impl_native_csr_word!(u32);
impl_native_csr_word!(u64);
impl_native_csr_word!(usize);

/// Implements CSR word traits for one little-endian zerocopy storage word.
macro_rules! impl_little_endian_csr_word {
    ($word:ty, $index:ty) => {
        impl CsrWord for $word {
            type Index = $index;

            fn get(self) -> Self::Index {
                Self::get(self)
            }
        }

        impl sealed::CsrSnapshotWord for $word {}

        impl CsrSnapshotWord for $word {}
    };
}

impl_little_endian_csr_word!(U16<LE>, u16);
impl_little_endian_csr_word!(U32<LE>, u32);
impl_little_endian_csr_word!(U64<LE>, u64);
impl_little_endian_csr_word!(Usize<LE>, usize);

/// Little-endian zerocopy word usable when opening CSR data from a snapshot.
///
/// This marker keeps snapshot loading endian-safe: `from_snapshot` is available
/// for unaligned little-endian storage words, not native-endian primitives.
///
/// # Performance
///
/// `perf: unspecified`; this marker trait has no methods.
pub trait CsrSnapshotWord:
    sealed::CsrSnapshotWord + CsrWord + FromBytes + Immutable + KnownLayout
{
}

/// Snapshot-backed little-endian CSR graph alias.
///
/// The `Index` parameter selects the logical index width (`u16`, `u32`, `u64`,
/// or `usize`). The underlying borrowed storage word is the matching zerocopy
/// little-endian wrapper. `usize` snapshots are pointer-width-specific; prefer
/// fixed-width indexes for portable persisted snapshots.
///
/// # Performance
///
/// `perf: unspecified`; this alias carries no runtime cost.
pub type CsrSnapshotGraph<'view, Index> =
    CsrGraph<'view, Index, <Index as CsrIndex>::LittleEndianWord>;

/// Local node ID for [`CsrGraph`].
///
/// Values are dense handles in `0..node_count` for one validated CSR view. They
/// are topology-local IDs and are not stable across rebuilding or compaction
/// unless a higher layer defines that contract.
///
/// # Performance
///
/// Copying, comparing, ordering, hashing, and debug-formatting are `O(1)` when
/// the underlying index type provides those operations in `O(1)`.
#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CsrNodeId<Index>(pub Index);

impl<Index> fmt::Debug for CsrNodeId<Index>
where
    Index: fmt::Debug,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("CsrNodeId").field(&self.0).finish()
    }
}

/// Local edge ID for [`CsrGraph`].
///
/// Values are dense handles into the flat CSR target array. They are
/// topology-local IDs and are not stable across sorting, rebuilding, or
/// compaction unless a higher layer defines that contract.
///
/// # Performance
///
/// Copying, comparing, ordering, hashing, and debug-formatting are `O(1)` when
/// the underlying index type provides those operations in `O(1)`.
#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CsrEdgeId<Index>(pub Index);

impl<Index> fmt::Debug for CsrEdgeId<Index>
where
    Index: fmt::Debug,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("CsrEdgeId").field(&self.0).finish()
    }
}

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
    fn from_id(id: CsrNodeId<Index>) -> Self {
        Self {
            raw: id.0,
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
    fn from_id(id: CsrEdgeId<Index>) -> Self {
        Self {
            raw: id.0,
            slot: 0,
            state: PhantomData,
        }
    }
}

impl<Index> EdgeSlot<Checked, Index>
where
    Index: CsrIndex,
{
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
    fn from_csr_range_slot(slot: usize) -> Self {
        let raw = Index::from_usize(slot)
            .unwrap_or_else(|| unreachable!("checked CSR edge slot must fit index type"));
        Self::from_raw_slot(raw, slot)
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
    const fn id(&self) -> CsrEdgeId<Index> {
        CsrEdgeId(self.raw)
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

        let slot = EdgeSlot::from_csr_range_slot(self.start);
        self.start += 1;
        Some(slot)
    }
}

/// Borrowed compressed-sparse-row graph view.
///
/// The graph stores outgoing adjacency using `offsets[node]..offsets[node + 1]`
/// ranges into the flat `targets` slice. The view borrows both slices and does
/// not allocate. `Index` is the single host-endian logical index type used for
/// nodes, edges, offsets, and targets. `StorageWord` is how those indexes are
/// stored in the borrowed slices; for native slices it is the same as `Index`,
/// and for snapshots it is a matching little-endian zerocopy wrapper.
///
/// # Performance
///
/// Creating a validated view is `O(n + m)` for `n` nodes and `m` edges because
/// validation checks monotonic offsets and target bounds. Outgoing traversal for
/// one node is `O(1)` to create and `O(k)` to yield `k` outgoing edges.
#[derive(Clone, Copy, Debug)]
pub struct CsrGraph<'view, Index, StorageWord>
where
    Index: CsrIndex,
    StorageWord: CsrWord<Index = Index>,
{
    /// Number of nodes in the graph as the public logical index type.
    node_count: Index,
    /// Number of nodes cached as a validated `usize` slot bound.
    node_bound: usize,
    /// CSR offsets with length `node_count + 1`.
    offsets: &'view [StorageWord],
    /// Flat outgoing target node IDs.
    targets: &'view [StorageWord],
}

impl<'view, Index, StorageWord> CsrGraph<'view, Index, StorageWord>
where
    Index: CsrIndex,
    StorageWord: CsrWord<Index = Index>,
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
        node_count: Index,
        offsets: &'view [StorageWord],
        targets: &'view [StorageWord],
    ) -> Result<Self, CsrError<Index>> {
        let node_bound = node_count.to_usize()?;
        let expected_offsets = node_bound
            .checked_add(1)
            .ok_or(CsrError::OffsetLengthOverflow { node_count })?;

        if offsets.len() != expected_offsets {
            return Err(CsrError::OffsetLength {
                expected: expected_offsets,
                actual: offsets.len(),
            });
        }

        let mut previous = Index::ZERO;
        for (index, offset_word) in offsets.iter().copied().enumerate() {
            let offset = offset_word.get();
            if index == 0 && offset != Index::ZERO {
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
        let final_offset_usize = final_offset.to_usize()?;
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
    pub const fn offsets(&self) -> &'view [StorageWord] {
        self.offsets
    }

    /// Returns the borrowed CSR target slice.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub const fn targets(&self) -> &'view [StorageWord] {
        self.targets
    }

    /// Returns whether `node` is valid in this CSR view.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub fn contains_node(&self, node: CsrNodeId<Index>) -> bool {
        self.try_node_slot(node).is_some()
    }

    /// Returns whether `edge` is valid in this CSR view.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub fn contains_edge(&self, edge: CsrEdgeId<Index>) -> bool {
        self.try_edge_slot(edge).is_some()
    }

    /// Returns the target node for `edge` when `edge` is valid in this view.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub fn try_target(&self, edge: CsrEdgeId<Index>) -> Option<CsrNodeId<Index>> {
        self.try_edge_slot(edge)
            .map(|checked| self.target_node(checked))
    }

    /// Checks a public node ID and returns a checked node slot on success.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    fn try_node_slot(&self, node: CsrNodeId<Index>) -> Option<NodeSlot<Checked, Index>> {
        self.check_node_slot(NodeSlot::from_id(node))
    }

    /// Converts an unchecked node slot into a checked node slot.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    fn check_node_slot(
        &self,
        node: NodeSlot<Unchecked, Index>,
    ) -> Option<NodeSlot<Checked, Index>> {
        let slot = match node.raw.to_usize() {
            Ok(value) => value,
            Err(_error) => return None,
        };
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
    fn checked_node_slot(&self, node: CsrNodeId<Index>) -> NodeSlot<Checked, Index> {
        self.try_node_slot(node)
            .unwrap_or_else(|| panic!("CSR node ID {node:?} is invalid for this graph"))
    }

    /// Checks a public edge ID and returns a checked edge slot on success.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    fn try_edge_slot(&self, edge: CsrEdgeId<Index>) -> Option<EdgeSlot<Checked, Index>> {
        self.check_edge_slot(EdgeSlot::from_id(edge))
    }

    /// Converts an unchecked edge slot into a checked edge slot.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    fn check_edge_slot(
        &self,
        edge: EdgeSlot<Unchecked, Index>,
    ) -> Option<EdgeSlot<Checked, Index>> {
        let slot = match edge.raw.to_usize() {
            Ok(value) => value,
            Err(_error) => return None,
        };
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
    fn checked_edge_slot(&self, edge: CsrEdgeId<Index>) -> EdgeSlot<Checked, Index> {
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
    fn checked_offset_slot(offset: Index) -> usize {
        match offset.to_usize() {
            Ok(slot) => slot,
            Err(_error) => unreachable!("checked CSR offset must fit usize"),
        }
    }

    /// Returns the target node for a checked CSR edge slot.
    ///
    /// # Performance
    ///
    /// This method is `O(1)` for valid edge IDs from this view.
    fn target_node(&self, edge: EdgeSlot<Checked, Index>) -> CsrNodeId<Index> {
        CsrNodeId(self.targets[edge.index()].get())
    }

    /// Returns the start and end edge slots for a checked node.
    ///
    /// # Performance
    ///
    /// This method is `O(1)` for valid node IDs from this view.
    fn outgoing_range(&self, node: NodeSlot<Checked, Index>) -> EdgeRange<Checked, Index> {
        let index = node.index();
        EdgeRange::from_bounds(
            Self::checked_offset_slot(self.offsets[index].get()),
            Self::checked_offset_slot(self.offsets[index + 1].get()),
        )
    }
}

impl<'view, Index, StorageWord> CsrGraph<'view, Index, StorageWord>
where
    Index: CsrIndex,
    StorageWord: CsrSnapshotWord<Index = Index>,
{
    /// Builds a snapshot-backed CSR view from a validated [`Snapshot`].
    ///
    /// Reads the [`SNAPSHOT_KIND_CSR_OFFSETS`] and [`SNAPSHOT_KIND_CSR_TARGETS`]
    /// sections, borrows them as little-endian index words, derives
    /// `node_count` from `offsets.len() - 1`, and runs CSR-shape validation. The
    /// returned view borrows directly from the snapshot's byte slice — no
    /// copying. Use [`CsrSnapshotGraph`] to select a snapshot index width by its
    /// native type, for example `CsrSnapshotGraph<'_, u64>`.
    ///
    /// # Errors
    ///
    /// Returns [`CsrSnapshotError`] when either section is missing, cannot be
    /// viewed as the selected word width, is empty, has too many offsets for
    /// the selected index type, or fails CSR validation.
    ///
    /// # Performance
    ///
    /// This function is `O(s + n + m)` for `s` snapshot sections, `n` graph
    /// nodes, and `m` graph edges.
    pub fn from_snapshot(snapshot: &Snapshot<'view>) -> Result<Self, CsrSnapshotError<Index>> {
        let offsets_section = snapshot
            .section(SNAPSHOT_KIND_CSR_OFFSETS)
            .ok_or(CsrSnapshotError::MissingOffsets)?;
        let targets_section = snapshot
            .section(SNAPSHOT_KIND_CSR_TARGETS)
            .ok_or(CsrSnapshotError::MissingTargets)?;

        let offsets: &'view [StorageWord] = offsets_section
            .try_as_slice()
            .map_err(CsrSnapshotError::OffsetsView)?;
        let targets: &'view [StorageWord] = targets_section
            .try_as_slice()
            .map_err(CsrSnapshotError::TargetsView)?;

        if offsets.is_empty() {
            return Err(CsrSnapshotError::OffsetsEmpty);
        }

        let node_count_usize = offsets.len() - 1;
        let node_count =
            Index::from_usize(node_count_usize).ok_or(CsrSnapshotError::NodeCountOverflow {
                offsets_len: offsets.len(),
            })?;

        Ok(Self::validate(node_count, offsets, targets)?)
    }
}

impl<Index, StorageWord> TopologyBase for CsrGraph<'_, Index, StorageWord>
where
    Index: CsrIndex,
    StorageWord: CsrWord<Index = Index>,
{
    type ElementId = CsrNodeId<Index>;
    type RelationId = CsrEdgeId<Index>;
}

impl<Index, StorageWord> TopologyCounts for CsrGraph<'_, Index, StorageWord>
where
    Index: CsrIndex,
    StorageWord: CsrWord<Index = Index>,
{
    fn element_count(&self) -> usize {
        self.node_bound
    }

    fn relation_count(&self) -> usize {
        self.targets.len()
    }
}

impl<Index, StorageWord> GraphCounts for CsrGraph<'_, Index, StorageWord>
where
    Index: CsrIndex,
    StorageWord: CsrWord<Index = Index>,
{
}

impl<Index, StorageWord> ElementIndex for CsrGraph<'_, Index, StorageWord>
where
    Index: CsrIndex,
    StorageWord: CsrWord<Index = Index>,
{
    fn element_bound(&self) -> usize {
        self.node_bound
    }

    fn element_index(&self, element: CsrNodeId<Index>) -> usize {
        self.checked_node_slot(element).index()
    }
}

impl<Index, StorageWord> RelationIndex for CsrGraph<'_, Index, StorageWord>
where
    Index: CsrIndex,
    StorageWord: CsrWord<Index = Index>,
{
    fn relation_bound(&self) -> usize {
        self.targets.len()
    }

    fn relation_index(&self, relation: CsrEdgeId<Index>) -> usize {
        self.checked_edge_slot(relation).index()
    }
}

impl<Index, StorageWord> ContainsElement for CsrGraph<'_, Index, StorageWord>
where
    Index: CsrIndex,
    StorageWord: CsrWord<Index = Index>,
{
    fn contains_element(&self, element: CsrNodeId<Index>) -> bool {
        self.contains_node(element)
    }
}

impl<Index, StorageWord> ContainsRelation for CsrGraph<'_, Index, StorageWord>
where
    Index: CsrIndex,
    StorageWord: CsrWord<Index = Index>,
{
    fn contains_relation(&self, relation: CsrEdgeId<Index>) -> bool {
        self.contains_edge(relation)
    }
}

impl<Index, StorageWord> EdgeTargetGraph for CsrGraph<'_, Index, StorageWord>
where
    Index: CsrIndex,
    StorageWord: CsrWord<Index = Index>,
{
    fn target(&self, edge: CsrEdgeId<Index>) -> CsrNodeId<Index> {
        self.target_node(self.checked_edge_slot(edge))
    }
}

impl<Index, StorageWord> OutgoingGraph for CsrGraph<'_, Index, StorageWord>
where
    Index: CsrIndex,
    StorageWord: CsrWord<Index = Index>,
{
    type OutEdges<'view>
        = CsrOutEdges<Index>
    where
        Self: 'view;

    fn outgoing_edges(&self, node: CsrNodeId<Index>) -> Self::OutEdges<'_> {
        CsrOutEdges {
            range: self.outgoing_range(self.checked_node_slot(node)),
        }
    }
}

impl<Index, StorageWord> OutgoingEdgeCount for CsrGraph<'_, Index, StorageWord>
where
    Index: CsrIndex,
    StorageWord: CsrWord<Index = Index>,
{
    fn out_degree(&self, node: CsrNodeId<Index>) -> usize {
        self.outgoing_range(self.checked_node_slot(node)).len()
    }
}

impl<Index, StorageWord> ElementSuccessors for CsrGraph<'_, Index, StorageWord>
where
    Index: CsrIndex,
    StorageWord: CsrWord<Index = Index>,
{
    type Successors<'view>
        = CsrOutNeighbors<'view, StorageWord>
    where
        Self: 'view;

    fn element_successors(&self, node: CsrNodeId<Index>) -> Self::Successors<'_> {
        let range = self.outgoing_range(self.checked_node_slot(node));
        CsrOutNeighbors {
            targets: self.targets[range.as_range()].iter(),
        }
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
pub struct CsrOutNeighbors<'view, StorageWord> {
    /// Remaining target words in the outgoing range.
    targets: core::slice::Iter<'view, StorageWord>,
}

impl<StorageWord> Iterator for CsrOutNeighbors<'_, StorageWord>
where
    StorageWord: CsrWord,
{
    type Item = CsrNodeId<StorageWord::Index>;

    fn next(&mut self) -> Option<Self::Item> {
        self.targets.next().map(|word| CsrNodeId(word.get()))
    }
}

impl<StorageWord> ExactSizeIterator for CsrOutNeighbors<'_, StorageWord>
where
    StorageWord: CsrWord,
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
pub enum CsrError<Index> {
    /// `node_count + 1` overflowed `usize`.
    OffsetLengthOverflow {
        /// Node count that could not be converted to an offset length.
        node_count: Index,
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
        actual: Index,
    },
    /// Offsets were not monotonically increasing.
    NonMonotonicOffset {
        /// Offset index where monotonicity failed.
        index: usize,
        /// Previous offset value.
        previous: Index,
        /// Actual offset value at `index`.
        actual: Index,
    },
    /// Final offset does not match target slice length.
    FinalOffset {
        /// Final offset value.
        final_offset: Index,
        /// Target slice length.
        target_len: usize,
    },
    /// Target node ID is outside `0..node_count`.
    TargetOutOfRange {
        /// Target slice index containing the bad value.
        index: usize,
        /// Bad target node ID.
        target: Index,
        /// Number of nodes in the graph.
        node_count: Index,
    },
    /// An index value could not be represented as `usize` on this target.
    UsizeOverflow {
        /// Value that could not be represented as `usize`.
        value: Index,
    },
}

impl<Index> fmt::Display for CsrError<Index>
where
    Index: fmt::Display,
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
            Self::UsizeOverflow { value } => {
                write!(formatter, "CSR index value {value} does not fit usize")
            }
        }
    }
}

impl<Index> core::error::Error for CsrError<Index> where Index: fmt::Debug + fmt::Display {}

/// Error returned when a snapshot cannot be opened as a CSR graph.
///
/// # Performance
///
/// `perf: unspecified`; errors are returned only from snapshot-bound paths.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CsrSnapshotError<Index> {
    /// The snapshot has no [`SNAPSHOT_KIND_CSR_OFFSETS`] section.
    MissingOffsets,
    /// The snapshot has no [`SNAPSHOT_KIND_CSR_TARGETS`] section.
    MissingTargets,
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
    Csr(CsrError<Index>),
}

impl<Index> fmt::Display for CsrSnapshotError<Index>
where
    Index: fmt::Display,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingOffsets => formatter.write_str("snapshot has no CSR offsets section"),
            Self::MissingTargets => formatter.write_str("snapshot has no CSR targets section"),
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

impl<Index> core::error::Error for CsrSnapshotError<Index> where Index: fmt::Debug + fmt::Display {}

impl<Index> From<CsrError<Index>> for CsrSnapshotError<Index> {
    fn from(error: CsrError<Index>) -> Self {
        Self::Csr(error)
    }
}
