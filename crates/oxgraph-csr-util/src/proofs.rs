//! Kani proof harnesses for offset-integrity primitives.
//!
//! Each `#[kani::proof]` exercises a bounded scenario the predicates must
//! never violate: totality (no panic on arbitrary input) and the algebraic
//! contract of monotonicity, length, range, and final-offset checks.
//! Heavy gate, run under `cargo kani`.

#![cfg(kani)]

use crate::{
    OffsetIntegrityIssue, check_offset_section, check_offsets_monotonic, check_value_range,
};

/// `check_offsets_monotonic` is total on arrays of up to 4 `u32` words.
#[kani::proof]
#[kani::unwind(8)]
fn check_offsets_monotonic_total_u32_n4() {
    let offsets: [u32; 4] = kani::any();
    let take: usize = kani::any();
    kani::assume(take <= 4);
    let _ = check_offsets_monotonic(&offsets[..take]);
}

/// `check_offsets_monotonic` accepts only sequences with first offset zero
/// (when non-empty) and non-decreasing offsets.
#[kani::proof]
#[kani::unwind(8)]
fn check_offsets_monotonic_predicate_u32_n3() {
    let offsets: [u32; 3] = kani::any();
    let take: usize = kani::any();
    kani::assume(take <= 3);
    let slice = &offsets[..take];
    if check_offsets_monotonic(slice).is_ok() {
        if !slice.is_empty() {
            assert_eq!(slice[0], 0);
        }
        for index in 1..slice.len() {
            assert!(slice[index - 1] <= slice[index]);
        }
    }
}

/// `check_value_range` is total on arrays of up to 4 `u32` values.
#[kani::proof]
#[kani::unwind(8)]
fn check_value_range_total_u32_n4() {
    let values: [u32; 4] = kani::any();
    let take: usize = kani::any();
    kani::assume(take <= 4);
    let bound: usize = kani::any();
    let _ = check_value_range(&values[..take], bound);
}

/// On `Ok`, every value in `values[..take]` is strictly less than `bound`.
#[kani::proof]
#[kani::unwind(8)]
fn check_value_range_predicate_u32_n3() {
    let values: [u32; 3] = kani::any();
    let take: usize = kani::any();
    kani::assume(take <= 3);
    let bound: usize = kani::any();
    let slice = &values[..take];
    if check_value_range(slice, bound).is_ok() {
        for &value in slice {
            assert!((value as usize) < bound);
        }
    }
}

/// `check_offset_section` is total: returns `Ok` or an
/// [`OffsetIntegrityIssue`] on any input within the kani bounds.
#[kani::proof]
#[kani::unwind(8)]
fn check_offset_section_total_u32_n4() {
    let offsets: [u32; 4] = kani::any();
    let take: usize = kani::any();
    kani::assume(take <= 4);
    let expected_count: usize = kani::any();
    kani::assume(expected_count <= 4);
    let value_len: usize = kani::any();
    let _ = check_offset_section(&offsets[..take], expected_count, value_len);
}

/// On `Ok`, length is `expected_count + 1`, first offset is zero, the slice is
/// monotonic non-decreasing, and the final offset matches `value_len`.
#[kani::proof]
#[kani::unwind(8)]
fn check_offset_section_predicate_u32_n3() {
    let offsets: [u32; 4] = kani::any();
    let take: usize = kani::any();
    kani::assume(take <= 4);
    let expected_count: usize = kani::any();
    kani::assume(expected_count <= 3);
    let value_len: usize = kani::any();
    kani::assume(value_len <= u32::MAX as usize);
    let slice = &offsets[..take];
    if check_offset_section(slice, expected_count, value_len).is_ok() {
        assert_eq!(slice.len(), expected_count + 1);
        assert_eq!(slice[0], 0);
        for index in 1..slice.len() {
            assert!(slice[index - 1] <= slice[index]);
        }
        assert_eq!(slice[slice.len() - 1] as usize, value_len);
    }
}

/// `OffsetIntegrityIssue::FirstNonZero` is the rejection reason for offsets
/// that begin with a non-zero value.
#[kani::proof]
fn check_offsets_monotonic_first_zero_required() {
    let first: u32 = kani::any();
    kani::assume(first != 0);
    let result = check_offsets_monotonic(&[first]);
    match result {
        Err(OffsetIntegrityIssue::FirstNonZero { actual }) => {
            assert_eq!(actual, first as usize);
        }
        Err(_) => {}
        Ok(()) => kani::cover!(false, "non-zero first offset must be rejected"),
    }
}
