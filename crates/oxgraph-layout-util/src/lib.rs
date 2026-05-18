//! Shared layout primitives for `OxGraph` graph and hypergraph crates.
//!
//! Two responsibilities live here:
//!
//! - **Build-time** ([`BuildIndex`], [`id_to_slot`], [`slot_or_max`], [`index_from_usize`],
//!   [`build_offset_index`]): validate dense IDs against a known count, convert `usize` slots back
//!   into a typed index width, and flatten per-bucket payloads into CSR-style `(offsets, items)`
//!   pairs. Used by `oxgraph-graph-build` and `oxgraph-hyper-build`.
//! - **Read-time** ([`ZerocopyWord`], [`OffsetIntegrityIssue`], [`check_offsets_monotonic`],
//!   [`check_value_range`], [`check_offset_section`]): walk borrowed offset arrays at view-open
//!   time to enforce length matches `count + 1`, first offset is zero, offsets non-decreasing, the
//!   final offset matches the value-array length, and every value fits in the dense bound. Used by
//!   `oxgraph-csr` and `oxgraph-hyper-bcsr`.
//!
//! Helpers return small typed data enums ([`IdOutOfBounds`],
//! [`OffsetOverflow`], [`OffsetIntegrityIssue`]) instead of crate-specific
//! error types. Callers map the issue to their own typed error at the
//! boundary.
//!
//! `no_std + alloc` (build-time primitives need `Vec`). No public domain
//! semantics. No dependency on any other `oxgraph` crate.
// kani-skip: helpers loop over arbitrary slice lengths and allocate
// variable-sized buffers; proofs exercise the algebraic contract on bounded
// fixtures.
#![no_std]

extern crate alloc;

#[cfg(kani)]
extern crate kani;

use alloc::vec::Vec;
use core::{error::Error, fmt, hash::Hash};

use zerocopy::byteorder::{LE, U16, U32, U64};

/// Sealed module preventing external types from satisfying the in-crate
/// build/zerocopy traits.
mod sealed {
    /// Seals [`super::BuildIndex`] to the supported unsigned index widths.
    pub trait BuildIndex {}

    /// Seals [`super::ZerocopyWord`] to in-tree CSR and BCSR word types.
    pub trait ZerocopyWord {}
}

// ---------------------------------------------------------------------------
// Build-time primitives
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Read-time offset-integrity primitives
// ---------------------------------------------------------------------------

/// Borrowed offset or value word usable by offset-integrity primitives.
///
/// Sealed: both `CsrWord` (in `oxgraph-csr`) and `BcsrWord` (in
/// `oxgraph-hyper-bcsr`) opt in via in-tree `impl ZerocopyWord for ...`
/// blocks. External crates cannot satisfy this trait.
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
///
/// # Errors
///
/// Returns the first [`OffsetIntegrityIssue`] encountered.
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
        return Err(OffsetIntegrityIssue::Length {
            expected: usize::MAX,
            actual: offsets.len(),
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
