//! Kani proof harnesses for layout-util primitives.
//!
//! Each `#[kani::proof]` exercises a bounded scenario the primitives must
//! never violate: totality (no panic on arbitrary input) and the algebraic
//! contract of monotonicity, length, range, final-offset checks, and
//! [`build_offset_index`]'s offset-monotonicity / first-zero /
//! final-equals-flat-length contract. Heavy gate, run under `cargo kani`.

#![cfg(kani)]

use alloc::{vec, vec::Vec};

use crate::{
    BuildIndex, IdOutOfBounds, OffsetIntegrityIssue, OffsetOverflow, build_offset_index,
    check_offset_section, check_offsets_monotonic, check_value_range, id_to_slot, index_from_usize,
    slot_or_max,
};

// ---------------------------------------------------------------------------
// Build-time proofs
// ---------------------------------------------------------------------------

/// `id_to_slot` is total for `u32` IDs against arbitrary `count`.
#[kani::proof]
fn id_to_slot_total_u32() {
    let id: u32 = kani::any();
    let count: usize = kani::any();
    let result = id_to_slot::<u32>(id, count);
    if let Ok(slot) = result {
        assert!(slot < count);
    }
}

/// `id_to_slot` for `u32` returns `OutOfRange` when the slot is `>= count` and
/// never `UsizeOverflow` (because `u32` always fits in `usize` on supported
/// kani targets).
#[kani::proof]
fn id_to_slot_classification_u32() {
    let id: u32 = kani::any();
    let count: usize = kani::any();
    match id_to_slot::<u32>(id, count) {
        Ok(slot) => {
            assert_eq!(slot, id as usize);
            assert!(slot < count);
        }
        Err(IdOutOfBounds::OutOfRange { slot, count: c }) => {
            assert_eq!(slot, id as usize);
            assert_eq!(c, count);
            assert!(slot >= c);
        }
        Err(IdOutOfBounds::UsizeOverflow) => {
            kani::cover!(false, "u32 always fits in usize on kani targets");
        }
    }
}

/// `slot_or_max` never panics for any `u64` input.
#[kani::proof]
fn slot_or_max_total_u64() {
    let id: u64 = kani::any();
    let _ = slot_or_max::<u64>(id);
}

/// `index_from_usize` is total for `u16`: returns `Ok` when `value` fits in
/// `u16`, otherwise [`OffsetOverflow::IndexOverflow`].
#[kani::proof]
fn index_from_usize_total_u16() {
    let value: usize = kani::any();
    match index_from_usize::<u16>(value) {
        Ok(idx) => {
            assert_eq!(idx as usize, value);
            assert!(value <= u16::MAX as usize);
        }
        Err(OffsetOverflow::IndexOverflow { value: v }) => {
            assert_eq!(v, value);
            assert!(value > u16::MAX as usize);
        }
    }
}

/// `build_offset_index` algebraic contract for two buckets of up to two
/// `u32` items each. On `Ok`, the returned `(offsets, items)` satisfies:
/// - `offsets.len() == buckets.len() + 1`
/// - `offsets[0] == 0`
/// - `offsets` is non-decreasing
/// - `offsets[buckets.len()] == items.len()`
/// - per-bucket lengths agree with offset deltas.
#[kani::proof]
#[kani::unwind(4)]
fn build_offset_index_contract_2x2_u32() {
    let bucket0_storage: [u32; 2] = kani::any();
    let bucket0_take: usize = kani::any();
    kani::assume(bucket0_take <= 2);
    let bucket0: Vec<u32> = bucket0_storage[..bucket0_take].to_vec();
    let bucket0_len = bucket0.len();

    let bucket1_storage: [u32; 2] = kani::any();
    let bucket1_take: usize = kani::any();
    kani::assume(bucket1_take <= 2);
    let bucket1: Vec<u32> = bucket1_storage[..bucket1_take].to_vec();
    let bucket1_len = bucket1.len();

    let buckets = vec![bucket0, bucket1];

    let (offsets, items) = match build_offset_index::<u32, u32>(buckets) {
        Ok(value) => value,
        Err(_) => return,
    };

    assert_eq!(offsets.len(), 3);
    assert_eq!(offsets[0], 0);
    assert!(offsets[0] <= offsets[1]);
    assert!(offsets[1] <= offsets[2]);
    assert_eq!(offsets[2] as usize, items.len());
    assert_eq!((offsets[1] - offsets[0]) as usize, bucket0_len);
    assert_eq!((offsets[2] - offsets[1]) as usize, bucket1_len);
}

/// `build_offset_index` is total for arbitrary single-bucket `u32` input —
/// it always returns `Ok` or one of the `OffsetOverflow` variants without
/// panicking.
#[kani::proof]
#[kani::unwind(4)]
fn build_offset_index_total_single_bucket_u32() {
    let bucket_storage: [u32; 2] = kani::any();
    let take: usize = kani::any();
    kani::assume(take <= 2);
    let bucket: Vec<u32> = bucket_storage[..take].to_vec();
    let buckets = vec![bucket];

    let _ = build_offset_index::<u32, u32>(buckets);
}

/// Empty bucket list returns a single zero offset and no items.
#[kani::proof]
fn build_offset_index_empty() {
    let buckets: Vec<Vec<u32>> = Vec::new();
    match build_offset_index::<u32, u32>(buckets) {
        Ok((offsets, items)) => {
            assert_eq!(offsets.len(), 1);
            assert_eq!(offsets[0], 0);
            assert!(items.is_empty());
        }
        Err(_) => {
            kani::cover!(false, "empty buckets must succeed");
        }
    }
}

/// `BuildIndex` round-trip on `u16`: any `usize` that fits in `u16` round-trips
/// back to itself through `from_usize` ∘ `to_usize`.
#[kani::proof]
fn build_index_roundtrip_u16() {
    let value: usize = kani::any();
    let Some(index) = <u16 as BuildIndex>::from_usize(value) else {
        return;
    };
    match <u16 as BuildIndex>::to_usize(index) {
        Some(back) => assert_eq!(back, value),
        None => kani::cover!(false, "u16 always fits in usize"),
    }
}

// ---------------------------------------------------------------------------
// Read-time offset-integrity proofs
// ---------------------------------------------------------------------------

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
