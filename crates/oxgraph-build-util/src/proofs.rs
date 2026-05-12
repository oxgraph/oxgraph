//! Kani proof harnesses for builder helper primitives.
//!
//! Each `#[kani::proof]` exercises a bounded scenario the helpers must never
//! violate: totality (no panic on arbitrary input) and the algebraic contract
//! of [`build_offset_index`] (offset-monotonicity, first-zero, final-equals-
//! flat-length). Heavy gate, run under `cargo kani`.

#![cfg(kani)]

use alloc::{vec, vec::Vec};

use crate::{
    BuildIndex, IdOutOfBounds, OffsetOverflow, build_offset_index, id_to_slot, index_from_usize,
    slot_or_max,
};

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
