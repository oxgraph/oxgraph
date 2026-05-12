//! Offset-integrity primitives shared by oxgraph CSR and BCSR layouts.
//!
//! Both `oxgraph-csr` and `oxgraph-hyper-bcsr` walk borrowed offset arrays at
//! view-open time to enforce: length matches `count + 1`, first offset is
//! zero, offsets are non-decreasing, the final offset matches the value-array
//! length, and every value fits in the dense bound. The predicates are
//! identical; only the typed errors and section discriminators differ.
//!
//! This crate exposes the predicates as ordinary functions over a sealed
//! [`ZerocopyWord`] trait that both `CsrWord` and `BcsrWord` opt into. Each
//! caller maps the returned [`OffsetIntegrityIssue`] to its own typed error
//! at the boundary, adding any section discriminator the layout needs.
//!
//! `no_std`. No public domain semantics. No dependency on any other oxgraph
//! crate.
// kani-skip: predicates loop over arbitrary slice lengths; proofs exercise
// bounded fixtures of the algebraic contract.
#![no_std]

#[cfg(kani)]
extern crate kani;

use core::{error::Error, fmt};

use zerocopy::byteorder::{LE, U16, U32, U64};

/// Sealed module preventing external types from satisfying [`ZerocopyWord`].
mod sealed {
    /// Seals [`super::ZerocopyWord`] to in-tree CSR and BCSR word types.
    pub trait ZerocopyWord {}
}

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
