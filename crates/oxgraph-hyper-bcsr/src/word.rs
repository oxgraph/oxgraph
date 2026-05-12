//! Borrowed-section word abstraction for bipartite-CSR payloads.

use core::{fmt, hash::Hash};

use zerocopy::{
    FromBytes, Immutable, IntoBytes, KnownLayout,
    byteorder::{LE, U16, U32, U64},
};

use crate::snapshot::{
    SNAPSHOT_KIND_BCSR_HEAD_OFFSETS_U16, SNAPSHOT_KIND_BCSR_HEAD_OFFSETS_U32,
    SNAPSHOT_KIND_BCSR_HEAD_OFFSETS_U64, SNAPSHOT_KIND_BCSR_HEAD_PARTICIPANTS_U16,
    SNAPSHOT_KIND_BCSR_HEAD_PARTICIPANTS_U32, SNAPSHOT_KIND_BCSR_HEAD_PARTICIPANTS_U64,
    SNAPSHOT_KIND_BCSR_TAIL_OFFSETS_U16, SNAPSHOT_KIND_BCSR_TAIL_OFFSETS_U32,
    SNAPSHOT_KIND_BCSR_TAIL_OFFSETS_U64, SNAPSHOT_KIND_BCSR_TAIL_PARTICIPANTS_U16,
    SNAPSHOT_KIND_BCSR_TAIL_PARTICIPANTS_U32, SNAPSHOT_KIND_BCSR_TAIL_PARTICIPANTS_U64,
    SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_HYPEREDGES_U16,
    SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_HYPEREDGES_U32,
    SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_HYPEREDGES_U64,
    SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_OFFSETS_U16, SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_OFFSETS_U32,
    SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_OFFSETS_U64,
    SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_HYPEREDGES_U16,
    SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_HYPEREDGES_U32,
    SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_HYPEREDGES_U64,
    SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_OFFSETS_U16, SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_OFFSETS_U32,
    SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_OFFSETS_U64,
};

/// Private sealing traits for supported BCSR index and snapshot word types.
mod sealed {
    /// Seals [`super::BcsrIndex`] to built-in unsigned integer widths.
    pub trait BcsrIndex {}

    /// Seals [`super::BcsrSnapshotIndex`] to portable snapshot widths.
    pub trait BcsrSnapshotIndex {}

    /// Seals [`super::BcsrSnapshotWord`] to built-in little-endian words.
    pub trait BcsrSnapshotWord {}
}

/// Unsigned native index type usable in bipartite-CSR memory views.
///
/// Implementations are `u16`, `u32`, `u64`, and `usize`. Persisted snapshots
/// use the stricter [`BcsrSnapshotIndex`] trait and never use `usize`.
///
/// # Performance
///
/// Copying, comparing, hashing, formatting, and checked conversions are
/// expected to be `O(1)`.
pub trait BcsrIndex:
    sealed::BcsrIndex + Copy + Eq + Ord + fmt::Debug + fmt::Display + Hash + Sized
{
    /// Zero value for this index type.
    ///
    /// # Performance
    ///
    /// Reading this constant is `O(1)`.
    const ZERO: Self;

    /// Converts this index value to `usize` when representable on this target.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    fn to_usize(self) -> Option<usize>;

    /// Converts a `usize` to this index type when representable.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    fn from_usize(value: usize) -> Option<Self>;
}

/// Implements [`BcsrIndex`] for one unsigned integer type.
macro_rules! impl_bcsr_index {
    ($index:ty) => {
        impl sealed::BcsrIndex for $index {}

        impl BcsrIndex for $index {
            const ZERO: Self = 0;

            fn to_usize(self) -> Option<usize> {
                usize::try_from(self).ok()
            }

            fn from_usize(value: usize) -> Option<Self> {
                Self::try_from(value).ok()
            }
        }
    };
}

impl_bcsr_index!(u16);
impl_bcsr_index!(u32);
impl_bcsr_index!(u64);

impl sealed::BcsrIndex for usize {}

impl BcsrIndex for usize {
    const ZERO: Self = 0;

    fn to_usize(self) -> Option<usize> {
        Some(self)
    }

    fn from_usize(value: usize) -> Option<Self> {
        Some(value)
    }
}

/// Integer word usable in borrowed bipartite-CSR sections.
///
/// Native in-memory fixtures use primitive unsigned integers. Snapshot-backed
/// views use unaligned little-endian zerocopy wrappers and expose the same
/// host-endian logical index through [`Self::Index`].
///
/// Implementors are also `oxgraph_csr_util::ZerocopyWord`, which exposes the
/// shared `read_as_usize` predicate used by the layout-validation primitives
/// in `oxgraph-csr-util`. The set of `ZerocopyWord` types is sealed in
/// `oxgraph-csr-util`, so this supertrait does not widen the public surface.
///
/// # Performance
///
/// Reading a word is expected to be `O(1)`.
pub trait BcsrWord: Copy + oxgraph_csr_util::ZerocopyWord {
    /// Host-endian logical index decoded from this word.
    ///
    /// # Performance
    ///
    /// `perf: unspecified`; this associated type carries no runtime cost.
    type Index: BcsrIndex;

    /// Returns this bipartite-CSR word as a host-endian index.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    fn get(self) -> Self::Index;
}

/// Implements [`BcsrWord`] for one native index word.
macro_rules! impl_native_bcsr_word {
    ($index:ty) => {
        impl BcsrWord for $index {
            type Index = $index;

            fn get(self) -> Self::Index {
                self
            }
        }
    };
}

impl_native_bcsr_word!(u16);
impl_native_bcsr_word!(u32);
impl_native_bcsr_word!(u64);
impl_native_bcsr_word!(usize);

/// Little-endian zerocopy word usable when opening BCSR data from a snapshot.
///
/// # Performance
///
/// `perf: unspecified`; this marker trait has no methods.
pub trait BcsrSnapshotWord:
    sealed::BcsrSnapshotWord + BcsrWord + FromBytes + Immutable + IntoBytes + KnownLayout
{
}

/// Implements little-endian word traits for one storage word.
macro_rules! impl_little_endian_bcsr_word {
    ($word:ty, $index:ty) => {
        impl BcsrWord for $word {
            type Index = $index;

            fn get(self) -> Self::Index {
                Self::get(self)
            }
        }

        impl sealed::BcsrSnapshotWord for $word {}

        impl BcsrSnapshotWord for $word {}
    };
}

impl_little_endian_bcsr_word!(U16<LE>, u16);
impl_little_endian_bcsr_word!(U32<LE>, u32);
impl_little_endian_bcsr_word!(U64<LE>, u64);

/// Portable index width usable for persisted BCSR snapshot payloads.
///
/// `usize` deliberately does not implement this trait.
///
/// # Performance
///
/// `perf: unspecified`; implementations provide `O(1)` metadata access and
/// little-endian word conversion.
pub trait BcsrSnapshotIndex: sealed::BcsrSnapshotIndex + BcsrIndex {
    /// Little-endian zerocopy storage word for this logical index type.
    ///
    /// # Performance
    ///
    /// `perf: unspecified`; this associated type carries no runtime cost.
    type LittleEndianWord: BcsrSnapshotWord<Index = Self>;

    /// Head offsets section kind for this width.
    const HEAD_OFFSETS_KIND: u32;
    /// Head participants section kind for this width.
    const HEAD_PARTICIPANTS_KIND: u32;
    /// Tail offsets section kind for this width.
    const TAIL_OFFSETS_KIND: u32;
    /// Tail participants section kind for this width.
    const TAIL_PARTICIPANTS_KIND: u32;
    /// Vertex outgoing offsets section kind for this width.
    const VERTEX_OUTGOING_OFFSETS_KIND: u32;
    /// Vertex outgoing hyperedges section kind for this width.
    const VERTEX_OUTGOING_HYPEREDGES_KIND: u32;
    /// Vertex incoming offsets section kind for this width.
    const VERTEX_INCOMING_OFFSETS_KIND: u32;
    /// Vertex incoming hyperedges section kind for this width.
    const VERTEX_INCOMING_HYPEREDGES_KIND: u32;

    /// Converts this value into its little-endian BCSR snapshot word.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    fn to_le_word(self) -> Self::LittleEndianWord;
}

/// Implements [`BcsrSnapshotIndex`] for one portable snapshot width.
macro_rules! impl_bcsr_snapshot_index {
    (
        $index:ty,
        $word:ty,
        $head_offsets:expr,
        $head_participants:expr,
        $tail_offsets:expr,
        $tail_participants:expr,
        $vertex_outgoing_offsets:expr,
        $vertex_outgoing_hyperedges:expr,
        $vertex_incoming_offsets:expr,
        $vertex_incoming_hyperedges:expr
    ) => {
        impl sealed::BcsrSnapshotIndex for $index {}

        impl BcsrSnapshotIndex for $index {
            type LittleEndianWord = $word;

            const HEAD_OFFSETS_KIND: u32 = $head_offsets;
            const HEAD_PARTICIPANTS_KIND: u32 = $head_participants;
            const TAIL_OFFSETS_KIND: u32 = $tail_offsets;
            const TAIL_PARTICIPANTS_KIND: u32 = $tail_participants;
            const VERTEX_OUTGOING_OFFSETS_KIND: u32 = $vertex_outgoing_offsets;
            const VERTEX_OUTGOING_HYPEREDGES_KIND: u32 = $vertex_outgoing_hyperedges;
            const VERTEX_INCOMING_OFFSETS_KIND: u32 = $vertex_incoming_offsets;
            const VERTEX_INCOMING_HYPEREDGES_KIND: u32 = $vertex_incoming_hyperedges;

            fn to_le_word(self) -> Self::LittleEndianWord {
                <$word>::new(self)
            }
        }
    };
}

impl_bcsr_snapshot_index!(
    u16,
    U16<LE>,
    SNAPSHOT_KIND_BCSR_HEAD_OFFSETS_U16,
    SNAPSHOT_KIND_BCSR_HEAD_PARTICIPANTS_U16,
    SNAPSHOT_KIND_BCSR_TAIL_OFFSETS_U16,
    SNAPSHOT_KIND_BCSR_TAIL_PARTICIPANTS_U16,
    SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_OFFSETS_U16,
    SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_HYPEREDGES_U16,
    SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_OFFSETS_U16,
    SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_HYPEREDGES_U16
);
impl_bcsr_snapshot_index!(
    u32,
    U32<LE>,
    SNAPSHOT_KIND_BCSR_HEAD_OFFSETS_U32,
    SNAPSHOT_KIND_BCSR_HEAD_PARTICIPANTS_U32,
    SNAPSHOT_KIND_BCSR_TAIL_OFFSETS_U32,
    SNAPSHOT_KIND_BCSR_TAIL_PARTICIPANTS_U32,
    SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_OFFSETS_U32,
    SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_HYPEREDGES_U32,
    SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_OFFSETS_U32,
    SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_HYPEREDGES_U32
);
impl_bcsr_snapshot_index!(
    u64,
    U64<LE>,
    SNAPSHOT_KIND_BCSR_HEAD_OFFSETS_U64,
    SNAPSHOT_KIND_BCSR_HEAD_PARTICIPANTS_U64,
    SNAPSHOT_KIND_BCSR_TAIL_OFFSETS_U64,
    SNAPSHOT_KIND_BCSR_TAIL_PARTICIPANTS_U64,
    SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_OFFSETS_U64,
    SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_HYPEREDGES_U64,
    SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_OFFSETS_U64,
    SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_HYPEREDGES_U64
);
