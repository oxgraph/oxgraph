//! Read-time offset-integrity primitives.
//!
//! View-open validation walks borrowed offset and value arrays
//! ([`check_offsets_monotonic`], [`check_value_range`],
//! [`check_offset_section`]) and converts already-validated indexes
//! infallibly ([`index_to_usize_validated`], [`usize_to_index_validated`]).
//!
//! Helpers return the small typed [`OffsetIntegrityIssue`] enum instead of
//! crate-specific error types; callers map the issue to their own typed error
//! at the boundary.

use core::{error::Error, fmt};

use crate::{LayoutIndex, ZerocopyWord};

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
