//! Shared layout primitives for `OxGraph` graph and hypergraph crates.
//!
//! Three responsibilities live here, one namespace each:
//!
//! - **Index/word vocabulary** (crate root: [`LayoutIndex`], [`LayoutWord`],
//!   [`LayoutSnapshotWord`], [`SnapshotWidth`]): the single sealed set of dense index widths,
//!   native/little-endian storage words, and the width-to-LE-word bijection shared by every layout
//!   and snapshot crate. CSR and BCSR add only thin section-kind-bearing sub-traits on top.
//! - **Build-time** ([`build`]): validate dense IDs against a known count, convert `usize` slots
//!   back into a typed index width, flatten per-bucket payloads into CSR-style `(offsets, items)`
//!   pairs, and lower native index slices into explicit little-endian words.
//! - **Read-time** ([`integrity`]): walk borrowed offset arrays at view-open time and convert
//!   already-validated indexes infallibly.
//!
//! The build and integrity helpers are deliberately not re-exported at the
//! crate root: the root namespace carries only the vocabulary every layer is
//! parameterized over, while builder and validation internals stay behind
//! their module names.
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

use core::{fmt, hash::Hash, iter::FusedIterator, marker::PhantomData};

use zerocopy::{
    FromBytes, Immutable, IntoBytes, KnownLayout, Unaligned,
    byteorder::{LE, U16, U32, U64},
};

pub mod build;
pub mod integrity;
#[cfg(feature = "alloc")]
pub mod keys;

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

#[cfg(kani)]
mod proofs;
