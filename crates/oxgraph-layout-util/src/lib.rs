//! Shared layout primitives for `OxGraph` graph and hypergraph crates.
//!
//! Three responsibilities live here:
//!
//! - **Index/word vocabulary** ([`LayoutIndex`], [`LayoutWord`], [`LayoutSnapshotWord`],
//!   [`SnapshotWidth`]): the single sealed set of dense index widths, native/little-endian storage
//!   words, and the width-to-LE-word bijection shared by every layout and snapshot crate. CSR and
//!   BCSR add only thin section-kind-bearing sub-traits on top.
//! - **Build-time** ([`id_to_slot`], [`slot_or_max`], [`index_from_usize`], [`build_offset_index`],
//!   [`slice_to_le`], [`map_offset_overflow`]): validate dense IDs against a known count, convert
//!   `usize` slots back into a typed index width, flatten per-bucket payloads into CSR-style
//!   `(offsets, items)` pairs, and lower native index slices into explicit little-endian words.
//! - **Read-time** ([`OffsetIntegrityIssue`], [`check_offsets_monotonic`], [`check_value_range`],
//!   [`check_offset_section`], [`index_to_usize_validated`], [`usize_to_index_validated`]): walk
//!   borrowed offset arrays at view-open time and convert already-validated indexes infallibly.
//!
//! Helpers return small typed data enums ([`IdOutOfBounds`], [`OffsetOverflow`],
//! [`OffsetIntegrityIssue`]) instead of crate-specific error types. Callers map
//! the issue to their own typed error at the boundary.
//!
//! [`LocalId`] is the one generic local-handle newtype every layout crate
//! aliases for its node/edge/vertex/hyperedge/incidence identities, and
//! [`IdSlice`] is the one slice-to-handle iterator they all reuse.
//!
//! `no_std + alloc` (build-time primitives need `Vec`). No public domain
//! semantics. No dependency on any other `oxgraph` crate.
// kani-skip: helpers loop over arbitrary slice lengths and allocate
// variable-sized buffers; proofs exercise the algebraic contract on bounded
// fixtures.
#![no_std]

#[cfg(any(feature = "alloc", kani))]
extern crate alloc;

#[cfg(kani)]
extern crate kani;

#[cfg(any(feature = "alloc", kani))]
use alloc::vec::Vec;
use core::{error::Error, fmt, hash::Hash, iter::FusedIterator, marker::PhantomData};

use zerocopy::{
    FromBytes, Immutable, IntoBytes, KnownLayout, Unaligned,
    byteorder::{LE, U16, U32, U64},
};

// ---------------------------------------------------------------------------
// CRC-32C (Castagnoli)
// ---------------------------------------------------------------------------

/// Lookup table for the byte-at-a-time reflected CRC-32C computation.
///
/// Generated at compile time from the reflected Castagnoli polynomial
/// `0x82F6_3B78` (bit-reversal of `0x1EDC_6F41`).
const CRC32C_TABLE: [u32; 256] = build_crc32c_table();

/// Builds the 256-entry reflected CRC-32C table at compile time.
const fn build_crc32c_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut index: usize = 0;
    let mut seed: u32 = 0;
    while index < 256 {
        let mut crc = seed;
        let mut bit = 0;
        while bit < 8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0x82F6_3B78 & mask);
            bit += 1;
        }
        table[index] = crc;
        index += 1;
        seed += 1;
    }
    table
}

/// Continues a CRC-32C (Castagnoli, polynomial `0x1EDC_6F41`) checksum over
/// `bytes`, seeded with the result of a prior call (seed `0` starts a fresh
/// checksum).
///
/// This is a portable, table-driven software implementation provided so
/// `no_std` layout crates can produce checksummed snapshot containers without
/// a `std`-only CRC dependency. It satisfies the continuation law
/// `crc32c_append(crc32c_append(0, a), b) == crc32c_append(0, ab)`, matching
/// the `crc32c` crate's `crc32c_append`, and `crc32c_append(0, b"") == 0`.
/// `std` consumers on hot write paths may prefer a hardware-accelerated
/// implementation (e.g. the `crc32c` crate) — any continuation-style CRC-32C
/// is byte-for-byte interchangeable with this one.
///
/// # Performance
///
/// This function is `O(bytes.len())` (one table lookup per byte; no
/// allocation).
#[must_use]
pub fn crc32c_append(crc: u32, bytes: &[u8]) -> u32 {
    let mut state = !crc;
    for &byte in bytes {
        let index = usize::from(state.to_le_bytes()[0] ^ byte);
        state = (state >> 8) ^ CRC32C_TABLE[index];
    }
    !state
}

/// Sealed module preventing external types from satisfying the in-crate
/// index/word traits.
mod sealed {
    /// Seals [`super::LayoutIndex`] to the supported unsigned index widths.
    pub trait LayoutIndex {}

    /// Seals [`super::ZerocopyWord`] to in-tree native and little-endian words.
    pub trait ZerocopyWord {}

    /// Seals [`super::LayoutSnapshotWord`] to little-endian storage words.
    pub trait LayoutSnapshotWord {}

    /// Seals [`super::SnapshotWidth`] to the persisted unsigned widths.
    pub trait SnapshotWidth {}

    /// Seals [`super::Axis`] to the in-tree local-handle axis markers.
    pub trait Axis {}
}

// ---------------------------------------------------------------------------
// Index / word vocabulary
// ---------------------------------------------------------------------------

/// Unsigned dense ID width usable by graph and hypergraph layouts and builders.
///
/// This is the single index-width contract for the whole substrate: it merges
/// what `oxgraph-csr` and `oxgraph-hyper-bcsr` previously duplicated as
/// `CsrIndex`/`BcsrIndex`. `usize` is included for native build-time indices;
/// persisted widths additionally implement [`SnapshotWidth`].
///
/// # Performance
///
/// Implementations perform checked conversions in `O(1)`.
pub trait LayoutIndex:
    sealed::LayoutIndex + Copy + Eq + Ord + fmt::Debug + fmt::Display + Hash + Sized
{
    /// The additive identity for this width (the zero offset / first slot).
    const ZERO: Self;

    /// Converts this ID to `usize` when representable on the current target.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    fn to_usize(self) -> Option<usize>;

    /// Converts a `usize` into this ID width when representable.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    fn from_usize(value: usize) -> Option<Self>;
}

/// Implements [`LayoutIndex`] for one unsigned width.
macro_rules! impl_layout_index {
    ($index:ty) => {
        impl sealed::LayoutIndex for $index {}

        impl LayoutIndex for $index {
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

impl_layout_index!(u16);
impl_layout_index!(u32);
impl_layout_index!(u64);

impl sealed::LayoutIndex for usize {}

impl LayoutIndex for usize {
    const ZERO: Self = 0;

    fn to_usize(self) -> Option<usize> {
        Some(self)
    }

    fn from_usize(value: usize) -> Option<Self> {
        Some(value)
    }
}

/// Borrowed offset or value word usable by offset-integrity primitives.
///
/// Sealed: native unsigned integers and little-endian storage words opt in via
/// the in-tree macros below. External crates cannot satisfy this trait.
///
/// # Performance
///
/// Reading a word is expected to be `O(1)`.
pub trait ZerocopyWord: sealed::ZerocopyWord + Copy {
    /// Reads this word's value as `usize`, or returns `None` when the value
    /// does not fit in `usize` on the current target.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    fn read_as_usize(self) -> Option<usize>;
}

/// Implements [`ZerocopyWord`] for one native unsigned integer type.
macro_rules! impl_native_zerocopy_word {
    ($word:ty) => {
        impl sealed::ZerocopyWord for $word {}

        impl ZerocopyWord for $word {
            fn read_as_usize(self) -> Option<usize> {
                usize::try_from(self).ok()
            }
        }
    };
}

impl_native_zerocopy_word!(u16);
impl_native_zerocopy_word!(u32);
impl_native_zerocopy_word!(u64);
impl_native_zerocopy_word!(usize);

/// Implements [`ZerocopyWord`] for one little-endian zerocopy storage word.
macro_rules! impl_le_zerocopy_word {
    ($word:ty) => {
        impl sealed::ZerocopyWord for $word {}

        impl ZerocopyWord for $word {
            fn read_as_usize(self) -> Option<usize> {
                usize::try_from(<$word>::get(self)).ok()
            }
        }
    };
}

impl_le_zerocopy_word!(U16<LE>);
impl_le_zerocopy_word!(U32<LE>);
impl_le_zerocopy_word!(U64<LE>);

/// A native-host or little-endian word carrying a typed dense [`LayoutIndex`].
///
/// This merges what `oxgraph-csr` and `oxgraph-hyper-bcsr` previously
/// duplicated as `CsrWord`/`BcsrWord`. The associated [`LayoutWord::Index`]
/// recovers the logical index value from either a native word (identity) or a
/// little-endian storage word (byte-order conversion), so a single [`IdSlice`]
/// drives both build-path and view-path iteration.
///
/// # Performance
///
/// [`LayoutWord::get`] is expected to be `O(1)`.
pub trait LayoutWord: Copy + ZerocopyWord {
    /// Logical dense index recovered from this word.
    type Index: LayoutIndex;

    /// Reads the logical index value out of this word.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    fn get(self) -> Self::Index;
}

/// Implements [`LayoutWord`] for one native unsigned integer (identity index).
macro_rules! impl_native_layout_word {
    ($word:ty) => {
        impl LayoutWord for $word {
            type Index = $word;

            fn get(self) -> Self::Index {
                self
            }
        }
    };
}

impl_native_layout_word!(u16);
impl_native_layout_word!(u32);
impl_native_layout_word!(u64);
impl_native_layout_word!(usize);

/// Implements [`LayoutWord`] for one little-endian word over a native index.
macro_rules! impl_le_layout_word {
    ($word:ty, $index:ty) => {
        impl LayoutWord for $word {
            type Index = $index;

            fn get(self) -> Self::Index {
                <$word>::get(self)
            }
        }
    };
}

impl_le_layout_word!(U16<LE>, u16);
impl_le_layout_word!(U32<LE>, u32);
impl_le_layout_word!(U64<LE>, u64);

/// A little-endian storage word usable in persisted snapshot payloads.
///
/// Sealed marker over [`LayoutWord`] plus the zerocopy byte-view bounds,
/// including [`Unaligned`] — persisted words are byte-aligned by design, so
/// generic wire records composed of them stay padding-free. Only the explicit
/// little-endian words implement it — native integers are excluded so
/// persisted payloads always carry a defined byte order.
///
/// # Performance
///
/// `perf: unspecified`; this is a marker trait.
pub trait LayoutSnapshotWord:
    sealed::LayoutSnapshotWord
    + LayoutWord
    + FromBytes
    + Immutable
    + IntoBytes
    + KnownLayout
    + Unaligned
{
}

/// Implements [`LayoutSnapshotWord`] for one little-endian storage word.
macro_rules! impl_layout_snapshot_word {
    ($word:ty) => {
        impl sealed::LayoutSnapshotWord for $word {}

        impl LayoutSnapshotWord for $word {}
    };
}

impl_layout_snapshot_word!(U16<LE>);
impl_layout_snapshot_word!(U32<LE>);
impl_layout_snapshot_word!(U64<LE>);

/// A persisted unsigned width with its little-endian storage word.
///
/// This is the single width-to-LE-word bijection shared by `oxgraph-csr`,
/// `oxgraph-hyper-bcsr`, and any future layout crate. `usize` deliberately does
/// not implement it: persisted snapshots are fixed-width.
///
/// # Performance
///
/// [`SnapshotWidth::to_le_word`] and [`SnapshotWidth::from_le_word`] are `O(1)`.
pub trait SnapshotWidth: sealed::SnapshotWidth + LayoutIndex {
    /// Two-bit width discriminant carried in a section kind's low bits:
    /// `0b00` = `u16`, `0b01` = `u32`, `0b10` = `u64` (`0b11` reserved).
    ///
    /// Layout crates declare 4-aligned `SNAPSHOT_KIND_*_BASE` constants and
    /// derive each persisted section kind as `BASE | WIDTH_CODE`, so one base
    /// constant covers every persisted width.
    const WIDTH_CODE: u32;

    /// Little-endian storage word for this width.
    type LittleEndianWord: LayoutSnapshotWord<Index = Self>;

    /// Lowers this logical index into its little-endian storage word.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    fn to_le_word(self) -> Self::LittleEndianWord;

    /// Recovers this logical index from a little-endian storage word.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    fn from_le_word(word: Self::LittleEndianWord) -> Self;
}

/// Implements [`SnapshotWidth`] for one width and its little-endian word.
macro_rules! impl_snapshot_width {
    ($index:ty, $word:ty, $width_code:expr) => {
        impl sealed::SnapshotWidth for $index {}

        impl SnapshotWidth for $index {
            const WIDTH_CODE: u32 = $width_code;

            type LittleEndianWord = $word;

            fn to_le_word(self) -> Self::LittleEndianWord {
                <$word>::new(self)
            }

            fn from_le_word(word: Self::LittleEndianWord) -> Self {
                <$word>::get(word)
            }
        }
    };
}

impl_snapshot_width!(u16, U16<LE>, 0b00);
impl_snapshot_width!(u32, U32<LE>, 0b01);
impl_snapshot_width!(u64, U64<LE>, 0b10);

// ---------------------------------------------------------------------------
// Local-handle newtype + slice iterator
// ---------------------------------------------------------------------------

/// Marker for a local-handle axis (node, edge, vertex, hyperedge, incidence).
///
/// Sealed: only the in-tree axis markers below implement it, so [`LocalId`]
/// cannot be branded with an arbitrary external type.
///
/// # Performance
///
/// `perf: unspecified`; this is a marker trait.
pub trait Axis: sealed::Axis {}

/// Defines one zero-sized local-handle axis marker.
macro_rules! define_axis {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name;

        impl sealed::Axis for $name {}

        impl Axis for $name {}
    };
}

define_axis!(
    /// Element axis for binary graphs (a node).
    NodeAxis
);
define_axis!(
    /// Relation axis for binary graphs (an edge).
    EdgeAxis
);
define_axis!(
    /// Element axis for hypergraphs (a vertex).
    VertexAxis
);
define_axis!(
    /// Relation axis for hypergraphs (a hyperedge).
    HyperedgeAxis
);
define_axis!(
    /// Incidence axis (a participant / endpoint).
    IncidenceAxis
);

/// A dense local handle: an axis-branded index value.
///
/// Every layout crate aliases this for its node/edge/vertex/hyperedge/incidence
/// identities so a built graph and its borrowed snapshot view yield the same
/// handle type. The `Axis` brand is a zero-sized `PhantomData<fn() -> A>` so the
/// handle stays `Copy`, `Send`, and `Sync` regardless of the marker, and
/// satisfies the `TopologyId` blanket bound when `Width` does.
///
/// # Performance
///
/// Copy, compare, order, hash, and debug-format are `O(1)`.
pub struct LocalId<A, Width> {
    /// The raw dense index value.
    value: Width,
    /// Zero-sized axis brand; `fn() -> A` keeps the handle `Send`/`Sync`.
    axis: PhantomData<fn() -> A>,
}

impl<A, Width> LocalId<A, Width> {
    /// Wraps a raw index value as an axis-branded handle.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[inline]
    #[must_use]
    pub const fn new(value: Width) -> Self {
        Self {
            value,
            axis: PhantomData,
        }
    }
}

impl<A, Width: Copy> LocalId<A, Width> {
    /// Returns the raw index value of this handle.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[inline]
    #[must_use]
    pub const fn get(self) -> Width {
        self.value
    }
}

impl<A, Width> From<Width> for LocalId<A, Width> {
    #[inline]
    fn from(value: Width) -> Self {
        Self::new(value)
    }
}

impl<A, Width: Clone> Clone for LocalId<A, Width> {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            value: self.value.clone(),
            axis: PhantomData,
        }
    }
}

impl<A, Width: Copy> Copy for LocalId<A, Width> {}

impl<A, Width: PartialEq> PartialEq for LocalId<A, Width> {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

impl<A, Width: Eq> Eq for LocalId<A, Width> {}

impl<A, Width: PartialOrd> PartialOrd for LocalId<A, Width> {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        self.value.partial_cmp(&other.value)
    }
}

impl<A, Width: Ord> Ord for LocalId<A, Width> {
    #[inline]
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.value.cmp(&other.value)
    }
}

impl<A, Width: Hash> Hash for LocalId<A, Width> {
    #[inline]
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.value.hash(state);
    }
}

impl<A, Width: fmt::Debug> fmt::Debug for LocalId<A, Width> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("LocalId").field(&self.value).finish()
    }
}

impl<A, Width: fmt::Display> fmt::Display for LocalId<A, Width> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.value.fmt(formatter)
    }
}

/// Iterator that maps a borrowed slice of [`LayoutWord`]s into axis-branded
/// handles, recovering each logical index and wrapping it with `Id::from`.
///
/// This is the single slice-to-handle iterator shared by every layout crate's
/// build-path (native words) and view-path (little-endian words).
///
/// # Performance
///
/// Advancing is `O(1)`; the iterator borrows and never allocates.
#[derive(Clone, Debug)]
pub struct IdSlice<'view, W: LayoutWord, Id> {
    /// Borrowed iterator over the backing word slice.
    inner: core::slice::Iter<'view, W>,
    /// Zero-sized brand for the produced handle type.
    id: PhantomData<fn() -> Id>,
}

impl<'view, W: LayoutWord, Id> IdSlice<'view, W, Id> {
    /// Creates an [`IdSlice`] over a borrowed word slice.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[inline]
    #[must_use]
    pub fn new(slice: &'view [W]) -> Self {
        Self {
            inner: slice.iter(),
            id: PhantomData,
        }
    }
}

impl<W: LayoutWord, Id> Iterator for IdSlice<'_, W, Id>
where
    Id: From<W::Index>,
{
    type Item = Id;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|word| Id::from(word.get()))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<W: LayoutWord, Id> ExactSizeIterator for IdSlice<'_, W, Id>
where
    Id: From<W::Index>,
{
    #[inline]
    fn len(&self) -> usize {
        self.inner.len()
    }
}

impl<W: LayoutWord, Id> DoubleEndedIterator for IdSlice<'_, W, Id>
where
    Id: From<W::Index>,
{
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        self.inner.next_back().map(|word| Id::from(word.get()))
    }
}

impl<W: LayoutWord, Id> FusedIterator for IdSlice<'_, W, Id> where Id: From<W::Index> {}

// ---------------------------------------------------------------------------
// Build-time primitives
// ---------------------------------------------------------------------------

/// Reasons an ID failed dense bounds validation.
///
/// # Performance
///
/// Formatting is `O(message length)`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum IdOutOfBounds {
    /// The ID's value did not fit in `usize` on the current target.
    UsizeOverflow,
    /// The ID's slot was greater than or equal to the dense count.
    OutOfRange {
        /// Slot derived from the ID.
        slot: usize,
        /// Exclusive upper bound for the slot.
        count: usize,
    },
}

impl fmt::Display for IdOutOfBounds {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UsizeOverflow => formatter.write_str("ID value did not fit in usize"),
            Self::OutOfRange { slot, count } => write!(
                formatter,
                "ID slot {slot} is not less than the dense bound {count}"
            ),
        }
    }
}

impl Error for IdOutOfBounds {}

/// Reasons a `usize` could not be represented in a target index width during
/// builder offset construction.
///
/// # Performance
///
/// Formatting is `O(message length)`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum OffsetOverflow {
    /// A `usize` value did not fit in the target [`LayoutIndex`] width.
    IndexOverflow {
        /// Value that did not fit.
        value: usize,
    },
}

impl fmt::Display for OffsetOverflow {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IndexOverflow { value } => {
                write!(
                    formatter,
                    "value {value} does not fit in the target index width"
                )
            }
        }
    }
}

impl Error for OffsetOverflow {}

/// Maps an [`OffsetOverflow`] into a caller-chosen error via `on_overflow`.
///
/// Replaces the per-crate `map_offset_overflow` copies: each builder crate
/// passes a closure constructing its own typed `IdOverflow { value }` variant.
///
/// # Performance
///
/// This function is `O(1)`.
#[inline]
pub fn map_offset_overflow<E>(error: OffsetOverflow, on_overflow: impl FnOnce(usize) -> E) -> E {
    match error {
        OffsetOverflow::IndexOverflow { value } => on_overflow(value),
    }
}

/// Allocates the next dense ID from `counter`: returns `counter`'s current
/// value as an `Index` and advances the counter by one.
///
/// Replaces the per-builder allocation copies: each builder passes a closure
/// constructing its own typed `IdOverflow { value }` variant. The error value
/// is the count that failed to fit the index width, or `usize::MAX` when the
/// counter itself can no longer advance.
///
/// # Errors
///
/// Returns `on_overflow(*counter)` when the current count does not fit
/// `Index`, and `on_overflow(usize::MAX)` when the counter would overflow
/// `usize`.
///
/// # Performance
///
/// This function is `O(1)`.
#[inline]
pub fn next_dense_index<Index: LayoutIndex, E>(
    counter: &mut usize,
    on_overflow: impl Fn(usize) -> E,
) -> Result<Index, E> {
    let id = Index::from_usize(*counter).ok_or_else(|| on_overflow(*counter))?;
    *counter = counter
        .checked_add(1)
        .ok_or_else(|| on_overflow(usize::MAX))?;
    Ok(id)
}

/// Validates that `id`'s `usize` representation is less than `count`.
///
/// # Errors
///
/// Returns [`IdOutOfBounds::UsizeOverflow`] when the ID does not fit in
/// `usize` on the current target, and [`IdOutOfBounds::OutOfRange`] when the
/// slot is not less than `count`.
///
/// # Performance
///
/// This function is `O(1)`.
#[inline]
pub fn id_to_slot<I: LayoutIndex>(id: I, count: usize) -> Result<usize, IdOutOfBounds> {
    let slot = id.to_usize().ok_or(IdOutOfBounds::UsizeOverflow)?;
    if slot < count {
        Ok(slot)
    } else {
        Err(IdOutOfBounds::OutOfRange { slot, count })
    }
}

/// Returns `id`'s `usize` representation, or `usize::MAX` when it does not
/// fit in `usize` on the current target.
///
/// Used for fallback display and for slot lookups guarded elsewhere by
/// [`id_to_slot`]. The `usize::MAX` sentinel is safe because callers compare
/// against an upper bound less than `usize::MAX` before indexing.
///
/// # Performance
///
/// This function is `O(1)`.
#[inline]
#[must_use]
pub fn slot_or_max<I: LayoutIndex>(id: I) -> usize {
    id.to_usize().unwrap_or(usize::MAX)
}

/// Converts a `usize` value into the target index width.
///
/// # Errors
///
/// Returns [`OffsetOverflow::IndexOverflow`] when `value` does not fit.
///
/// # Performance
///
/// This function is `O(1)`.
#[inline]
pub fn index_from_usize<O: LayoutIndex>(value: usize) -> Result<O, OffsetOverflow> {
    O::from_usize(value).ok_or(OffsetOverflow::IndexOverflow { value })
}

/// Converts an already-validated index into `usize`.
///
/// Returns `None` only when the value does not fit in `usize`. Call sites that
/// have proven the value fits (because a prior validation pass enforced it)
/// should `unwrap_or_else(|| unreachable!("validated …"))` with a proof comment;
/// this helper deliberately does not embed that `unreachable!`, because the
/// proof obligation belongs to the validated call site, not the shared helper.
///
/// # Performance
///
/// This function is `O(1)`.
#[inline]
#[must_use]
pub fn index_to_usize_validated<I: LayoutIndex>(value: I) -> Option<usize> {
    value.to_usize()
}

/// Converts an already-validated `usize` slot into the target index width.
///
/// Returns `None` only when the value does not fit. See
/// [`index_to_usize_validated`] for the proof-obligation convention.
///
/// # Performance
///
/// This function is `O(1)`.
#[inline]
#[must_use]
pub fn usize_to_index_validated<I: LayoutIndex>(value: usize) -> Option<I> {
    I::from_usize(value)
}

/// Lowers a native index slice into explicit little-endian storage words.
///
/// Replaces the per-crate `*_slice_to_le` copies in the CSR and BCSR builders.
///
/// # Performance
///
/// This function is `O(values.len())`.
#[cfg(any(feature = "alloc", kani))]
#[must_use]
pub fn slice_to_le<W: SnapshotWidth>(values: &[W]) -> Vec<W::LittleEndianWord> {
    values.iter().copied().map(W::to_le_word).collect()
}

/// Flattens per-bucket payloads into a `(offsets, items)` pair.
///
/// The returned `offsets` vector has length `buckets.len() + 1`. `offsets[0]`
/// is zero, `offsets[i + 1] - offsets[i]` equals the i-th bucket's length, and
/// `offsets[buckets.len()]` equals `items.len()`. Items appear in input order
/// within each bucket and buckets are concatenated in input order.
///
/// # Errors
///
/// Returns [`OffsetOverflow::IndexOverflow`] when any cumulative offset does
/// not fit in the target index width.
///
/// # Performance
///
/// This function is `O(n)` where `n` is the total item count across all
/// buckets. Allocation matches a single-pass extend-and-grow; no second pass
/// is performed.
#[cfg(any(feature = "alloc", kani))]
pub fn build_offset_index<O, T>(buckets: Vec<Vec<T>>) -> Result<(Vec<O>, Vec<T>), OffsetOverflow>
where
    O: LayoutIndex,
{
    let mut offsets = Vec::with_capacity(buckets.len() + 1);
    let mut items: Vec<T> = Vec::new();
    offsets.push(index_from_usize::<O>(0)?);
    for bucket in buckets {
        items.extend(bucket);
        offsets.push(index_from_usize::<O>(items.len())?);
    }
    Ok((offsets, items))
}

// ---------------------------------------------------------------------------
// Read-time offset-integrity primitives
// ---------------------------------------------------------------------------

/// Reasons a borrowed offset or value array failed structural validation.
///
/// # Performance
///
/// Formatting is `O(message length)`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum OffsetIntegrityIssue {
    /// Length did not match `count + 1`.
    Length {
        /// Expected length (`count + 1`).
        expected: usize,
        /// Observed length.
        actual: usize,
    },
    /// `expected_count + 1` overflowed `usize`, so no valid length exists.
    CountOverflow {
        /// The element count whose successor overflowed.
        count: usize,
    },
    /// `offsets[0]` was not zero.
    FirstNonZero {
        /// Observed first offset.
        actual: usize,
    },
    /// `offsets[index] < offsets[index - 1]`.
    NonMonotonic {
        /// Index where monotonicity failed.
        index: usize,
        /// Offset at `index - 1`.
        previous: usize,
        /// Offset at `index`.
        actual: usize,
    },
    /// `offsets[count]` did not match `value_len`.
    FinalMismatch {
        /// Observed final offset.
        final_offset: usize,
        /// Length of the values array.
        value_len: usize,
    },
    /// A value at `index` was not less than the dense `bound`.
    ValueOutOfRange {
        /// Index of the offending value.
        index: usize,
        /// Observed value.
        value: usize,
        /// Exclusive upper bound.
        bound: usize,
    },
    /// A word's value at `index` did not fit in `usize` on the current target.
    UsizeOverflow {
        /// Slice position of the offending word.
        index: usize,
    },
}

impl fmt::Display for OffsetIntegrityIssue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Length { expected, actual } => write!(
                formatter,
                "offsets length {actual} does not match expected {expected}"
            ),
            Self::CountOverflow { count } => {
                write!(formatter, "element count {count} + 1 overflows usize")
            }
            Self::FirstNonZero { actual } => {
                write!(formatter, "first offset {actual} must be zero")
            }
            Self::NonMonotonic {
                index,
                previous,
                actual,
            } => write!(
                formatter,
                "offsets[{index}] = {actual} is less than offsets[{}] = {previous}",
                index - 1
            ),
            Self::FinalMismatch {
                final_offset,
                value_len,
            } => write!(
                formatter,
                "final offset {final_offset} does not match values length {value_len}"
            ),
            Self::ValueOutOfRange {
                index,
                value,
                bound,
            } => write!(
                formatter,
                "values[{index}] = {value} is not less than bound {bound}"
            ),
            Self::UsizeOverflow { index } => write!(
                formatter,
                "word at slice index {index} did not fit in usize"
            ),
        }
    }
}

impl Error for OffsetIntegrityIssue {}

/// Verifies `offsets[0] == 0` and that `offsets` is non-decreasing.
///
/// # Errors
///
/// Returns [`OffsetIntegrityIssue::FirstNonZero`] when the first offset is
/// non-zero, [`OffsetIntegrityIssue::NonMonotonic`] when an offset decreases
/// from the previous, and [`OffsetIntegrityIssue::UsizeOverflow`] when a
/// word's value does not fit in `usize`.
///
/// # Performance
///
/// This function is `O(offsets.len())`.
pub fn check_offsets_monotonic<W: ZerocopyWord>(offsets: &[W]) -> Result<(), OffsetIntegrityIssue> {
    let mut previous: usize = 0;
    for (index, word) in offsets.iter().copied().enumerate() {
        let offset = word
            .read_as_usize()
            .ok_or(OffsetIntegrityIssue::UsizeOverflow { index })?;
        if index == 0 {
            if offset != 0 {
                return Err(OffsetIntegrityIssue::FirstNonZero { actual: offset });
            }
        } else if offset < previous {
            return Err(OffsetIntegrityIssue::NonMonotonic {
                index,
                previous,
                actual: offset,
            });
        }
        previous = offset;
    }
    Ok(())
}

/// Verifies every value in `values` is less than `bound`.
///
/// # Errors
///
/// Returns [`OffsetIntegrityIssue::ValueOutOfRange`] when a value is at or
/// above `bound`, and [`OffsetIntegrityIssue::UsizeOverflow`] when a word's
/// value does not fit in `usize`.
///
/// # Performance
///
/// This function is `O(values.len())`.
pub fn check_value_range<W: ZerocopyWord>(
    values: &[W],
    bound: usize,
) -> Result<(), OffsetIntegrityIssue> {
    for (index, word) in values.iter().copied().enumerate() {
        let value = word
            .read_as_usize()
            .ok_or(OffsetIntegrityIssue::UsizeOverflow { index })?;
        if value >= bound {
            return Err(OffsetIntegrityIssue::ValueOutOfRange {
                index,
                value,
                bound,
            });
        }
    }
    Ok(())
}

/// Validates one offset section against `expected_count` rows and a backing
/// values array of length `value_len`.
///
/// Performs four checks: length matches `expected_count + 1`, first offset is
/// zero, offsets are non-decreasing, and the final offset matches `value_len`.
/// Because offsets are monotonic and the final offset equals `value_len`, every
/// interior offset is implied to be `<= value_len`; the borrowed-view slicing in
/// CSR/BCSR relies on that consequence and the proofs assert it directly.
///
/// # Errors
///
/// Returns the first [`OffsetIntegrityIssue`] encountered. Reports
/// [`OffsetIntegrityIssue::CountOverflow`] when `expected_count + 1` overflows.
///
/// # Performance
///
/// This function is `O(offsets.len())`.
pub fn check_offset_section<W: ZerocopyWord>(
    offsets: &[W],
    expected_count: usize,
    value_len: usize,
) -> Result<(), OffsetIntegrityIssue> {
    let Some(expected) = expected_count.checked_add(1) else {
        return Err(OffsetIntegrityIssue::CountOverflow {
            count: expected_count,
        });
    };
    if offsets.len() != expected {
        return Err(OffsetIntegrityIssue::Length {
            expected,
            actual: offsets.len(),
        });
    }
    check_offsets_monotonic(offsets)?;
    let final_index = offsets.len() - 1;
    let final_offset = offsets[final_index]
        .read_as_usize()
        .ok_or(OffsetIntegrityIssue::UsizeOverflow { index: final_index })?;
    if final_offset != value_len {
        return Err(OffsetIntegrityIssue::FinalMismatch {
            final_offset,
            value_len,
        });
    }
    Ok(())
}

#[cfg(kani)]
mod proofs;
