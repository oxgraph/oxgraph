//! Build-time layout primitives.
//!
//! Builders validate dense IDs against a known count ([`id_to_slot`],
//! [`slot_or_max`]), convert `usize` slots back into a typed index width
//! ([`index_from_usize`], [`next_dense_index`]), flatten per-bucket payloads
//! into CSR-style `(offsets, items)` pairs ([`build_offset_index`]), and lower
//! native index slices into explicit little-endian words ([`slice_to_le`]).
//!
//! Helpers return small typed data enums ([`IdOutOfBounds`], [`OffsetOverflow`])
//! instead of crate-specific error types; callers map the issue to their own
//! typed error at the boundary ([`map_offset_overflow`]).

#[cfg(any(feature = "alloc", kani))]
use alloc::vec::Vec;
use core::{error::Error, fmt};

use crate::LayoutIndex;
#[cfg(any(feature = "alloc", kani))]
use crate::SnapshotWidth;

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
