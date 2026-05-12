//! Builder helper primitives shared by `OxGraph` graph and hypergraph builders.
//!
//! Both `oxgraph-graph-build` and `oxgraph-hyper-build` need to validate dense
//! IDs against a known count, convert `usize` slots back into a typed index
//! width, and flatten per-bucket payloads into CSR-style `(offsets, items)`
//! pairs. This crate owns the canonical [`BuildIndex`] sealed trait and
//! exposes those primitives as ordinary public functions over it.
//!
//! Helpers return small typed data enums ([`IdOutOfBounds`], [`OffsetOverflow`])
//! instead of crate-specific error types. Callers map the issue to their own
//! typed error variant at the boundary.
//!
//! `no_std + alloc`. No public domain semantics. No dependency on any other
//! oxgraph crate.
// kani-skip: helpers allocate variable-sized buffers; proofs exercise the
// algebraic contract of build_offset_index on bounded fixtures.
#![no_std]

extern crate alloc;

#[cfg(kani)]
extern crate kani;

use alloc::vec::Vec;
use core::{error::Error, fmt, hash::Hash};

/// Sealed trait module for builder index widths.
mod sealed {
    /// Seals [`super::BuildIndex`] to the supported unsigned index widths.
    pub trait BuildIndex {}
}

/// Unsigned dense ID width usable by graph and hypergraph builders.
///
/// # Performance
///
/// Implementations perform checked conversions in `O(1)`.
pub trait BuildIndex: sealed::BuildIndex + Copy + Eq + Ord + fmt::Debug + Hash {
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

/// Implements [`BuildIndex`] for one unsigned width.
macro_rules! impl_build_index {
    ($index:ty) => {
        impl sealed::BuildIndex for $index {}

        impl BuildIndex for $index {
            fn to_usize(self) -> Option<usize> {
                usize::try_from(self).ok()
            }

            fn from_usize(value: usize) -> Option<Self> {
                Self::try_from(value).ok()
            }
        }
    };
}

impl_build_index!(u16);
impl_build_index!(u32);
impl_build_index!(u64);

impl sealed::BuildIndex for usize {}

impl BuildIndex for usize {
    fn to_usize(self) -> Option<usize> {
        Some(self)
    }

    fn from_usize(value: usize) -> Option<Self> {
        Some(value)
    }
}

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
    /// A `usize` value did not fit in the target [`BuildIndex`] width.
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
pub fn id_to_slot<I: BuildIndex>(id: I, count: usize) -> Result<usize, IdOutOfBounds> {
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
pub fn slot_or_max<I: BuildIndex>(id: I) -> usize {
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
pub fn index_from_usize<O: BuildIndex>(value: usize) -> Result<O, OffsetOverflow> {
    O::from_usize(value).ok_or(OffsetOverflow::IndexOverflow { value })
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
pub fn build_offset_index<O, T>(buckets: Vec<Vec<T>>) -> Result<(Vec<O>, Vec<T>), OffsetOverflow>
where
    O: BuildIndex,
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

#[cfg(kani)]
mod proofs;
