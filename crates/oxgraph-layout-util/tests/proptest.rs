//! Property tests for layout-util primitives (build-time + read-time).

use oxgraph_layout_util::{
    LayoutIndex,
    build::{
        IdOutOfBounds, OffsetOverflow, build_offset_index, id_to_slot, index_from_usize,
        slot_or_max,
    },
    integrity::{
        OffsetIntegrityIssue, check_offset_section, check_offsets_monotonic, check_value_range,
    },
};
use proptest::{prelude::*, test_runner::TestCaseError};

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    // ---- build-time --------------------------------------------------------

    /// `id_to_slot` returns a slot strictly less than `count` on success and
    /// classifies failures correctly for `u32` IDs.
    #[test]
    fn id_to_slot_u32_classification(id: u32, count: usize) {
        match id_to_slot::<u32>(id, count) {
            Ok(slot) => {
                prop_assert_eq!(slot, id as usize);
                prop_assert!(slot < count);
            }
            Err(IdOutOfBounds::OutOfRange { slot, count: c }) => {
                prop_assert_eq!(slot, id as usize);
                prop_assert_eq!(c, count);
                prop_assert!(slot >= c);
            }
            Err(IdOutOfBounds::UsizeOverflow) => {
                return Err(TestCaseError::fail(
                    "u32 always fits in usize on supported targets",
                ));
            }
            Err(_) => {
                return Err(TestCaseError::fail("unexpected IdOutOfBounds variant"));
            }
        }
    }

    /// `id_to_slot` for `u64` may surface `UsizeOverflow` on 32-bit targets;
    /// otherwise the slot is exactly `id as usize` and `< count` on success.
    #[test]
    fn id_to_slot_u64_returns_typed_error_or_slot(id: u64, count: usize) {
        match id_to_slot::<u64>(id, count) {
            Ok(slot) => {
                prop_assert!(slot < count);
            }
            Err(IdOutOfBounds::OutOfRange { slot, count: c }) => {
                prop_assert_eq!(c, count);
                prop_assert!(slot >= c);
            }
            Err(IdOutOfBounds::UsizeOverflow) => {
                let fits = usize::try_from(id).is_ok();
                prop_assert!(!fits, "UsizeOverflow only for ids that exceed usize");
            }
            Err(_) => {
                return Err(TestCaseError::fail("unexpected IdOutOfBounds variant"));
            }
        }
    }

    /// `slot_or_max` never panics and matches the typed conversion when the
    /// value fits in `usize`.
    #[test]
    fn slot_or_max_total(id: u64) {
        let result = slot_or_max::<u64>(id);
        if let Some(value) = <u64 as LayoutIndex>::to_usize(id) {
            prop_assert_eq!(result, value);
        } else {
            prop_assert_eq!(result, usize::MAX);
        }
    }

    /// `index_from_usize::<u16>` succeeds iff the value fits in `u16`.
    #[test]
    fn index_from_usize_u16_classification(value: usize) {
        match index_from_usize::<u16>(value) {
            Ok(idx) => {
                prop_assert_eq!(idx as usize, value);
                prop_assert!(u16::try_from(value).is_ok());
            }
            Err(OffsetOverflow::IndexOverflow { value: v }) => {
                prop_assert_eq!(v, value);
                prop_assert!(u16::try_from(value).is_err());
            }
            Err(_) => {
                return Err(TestCaseError::fail("unexpected OffsetOverflow variant"));
            }
        }
    }

    /// `build_offset_index` algebraic contract: offsets are non-decreasing,
    /// start at 0, end at `items.len()`, and per-bucket lengths agree with
    /// offset deltas.
    #[test]
    fn build_offset_index_algebraic_contract_u32(
        buckets in prop::collection::vec(
            prop::collection::vec(any::<u32>(), 0..8),
            0..16,
        ),
    ) {
        let bucket_lens: Vec<usize> = buckets.iter().map(Vec::len).collect();
        let total: usize = bucket_lens.iter().sum();
        let result = build_offset_index::<u32, u32>(buckets);
        let (offsets, items) = match result {
            Ok(value) => value,
            Err(OffsetOverflow::IndexOverflow { value }) => {
                prop_assert!(value > u32::MAX as usize);
                return Ok(());
            }
            Err(_) => {
                return Err(TestCaseError::fail("unexpected OffsetOverflow variant"));
            }
        };
        prop_assert_eq!(offsets.len(), bucket_lens.len() + 1);
        prop_assert_eq!(offsets[0], 0);
        prop_assert_eq!(items.len(), total);
        prop_assert_eq!(offsets[offsets.len() - 1] as usize, total);
        for window in offsets.windows(2) {
            prop_assert!(window[0] <= window[1]);
        }
        for index in 0..bucket_lens.len() {
            let delta = (offsets[index + 1] - offsets[index]) as usize;
            prop_assert_eq!(delta, bucket_lens[index]);
        }
    }

    /// `LayoutIndex::from_usize` ∘ `LayoutIndex::to_usize` is identity for `u32`.
    #[test]
    fn build_index_roundtrip_u32(value in 0_usize..(u32::MAX as usize + 1)) {
        let Some(index) = <u32 as LayoutIndex>::from_usize(value) else {
            return Err(TestCaseError::fail("value should fit u32"));
        };
        let Some(back) = <u32 as LayoutIndex>::to_usize(index) else {
            return Err(TestCaseError::fail("u32 always fits usize"));
        };
        prop_assert_eq!(back, value);
    }

    // ---- read-time ---------------------------------------------------------

    /// `check_offsets_monotonic` accepts iff the first offset is zero (when
    /// non-empty) and the slice is non-decreasing.
    #[test]
    fn monotonic_classification(offsets in prop::collection::vec(any::<u32>(), 0..32)) {
        let result = check_offsets_monotonic(&offsets);
        let well_formed = if offsets.is_empty() {
            true
        } else {
            offsets[0] == 0 && offsets.windows(2).all(|w| w[0] <= w[1])
        };
        match (result, well_formed) {
            (Ok(()), true) | (Err(_), false) => {}
            (Ok(()), false) => {
                return Err(TestCaseError::fail("accepted ill-formed offsets"));
            }
            (Err(_), true) => {
                return Err(TestCaseError::fail("rejected well-formed offsets"));
            }
        }
    }

    /// `check_value_range` accepts iff every value is strictly less than `bound`.
    #[test]
    fn value_range_classification(
        values in prop::collection::vec(any::<u32>(), 0..32),
        bound in 0_usize..(u32::MAX as usize + 1),
    ) {
        let result = check_value_range(&values, bound);
        let well_formed = values.iter().all(|&v| (v as usize) < bound);
        match (result, well_formed) {
            (Err(OffsetIntegrityIssue::ValueOutOfRange { value, bound: b, .. }), false) => {
                prop_assert!(value >= b);
                prop_assert_eq!(b, bound);
            }
            (Ok(()), true) | (Err(_), false) => {}
            (Ok(()), false) => {
                return Err(TestCaseError::fail("accepted out-of-range value"));
            }
            (Err(_), true) => {
                return Err(TestCaseError::fail("rejected in-range values"));
            }
        }
    }

    /// `check_offset_section` accepts iff length matches `count + 1`, first
    /// offset is zero, slice is monotonic, and final offset matches `value_len`.
    #[test]
    fn section_classification(
        offsets in prop::collection::vec(any::<u32>(), 0..32),
        count in 0_usize..32,
        value_len in 0_usize..(u32::MAX as usize + 1),
    ) {
        let result = check_offset_section(&offsets, count, value_len);
        let length_ok = offsets.len() == count + 1;
        let first_ok = offsets.first().is_none_or(|&v| v == 0);
        let monotonic = offsets.windows(2).all(|w| w[0] <= w[1]);
        let final_ok = offsets
            .last()
            .is_none_or(|&v| v as usize == value_len);
        let well_formed = length_ok && first_ok && monotonic && final_ok;
        match (result, well_formed) {
            (Ok(()), true) | (Err(_), false) => {}
            (Ok(()), false) => {
                return Err(TestCaseError::fail("accepted ill-formed section"));
            }
            (Err(_), true) => {
                return Err(TestCaseError::fail("rejected well-formed section"));
            }
        }
    }

    /// Final offset's `usize` equals `value_len` on success — requires at
    /// least one row (count >= 1) when `value_len > 0` so the final offset is
    /// distinct from the first.
    #[test]
    fn section_final_offset_matches_value_len(
        count in 1_usize..16,
        value_len in 0_usize..(u32::MAX as usize),
    ) {
        let mut offsets = vec![0_u32; count + 1];
        let Ok(value) = u32::try_from(value_len) else {
            return Ok(());
        };
        if let Some(last) = offsets.last_mut() {
            *last = value;
        }
        let result = check_offset_section(&offsets, count, value_len);
        prop_assert!(result.is_ok());
    }
}

proptest! {
    /// `next_dense_index` yields every dense `u16` ID in order, then errors
    /// with the count that failed to fit once the width is exhausted.
    #[test]
    fn next_dense_index_yields_dense_u16_ids(start in 0_usize..(u16::MAX as usize)) {
        let mut counter = start;
        let id: u16 = oxgraph_layout_util::build::next_dense_index(&mut counter, |value| value)
            .expect("in-width count allocates");
        prop_assert_eq!(usize::from(id), start);
        prop_assert_eq!(counter, start + 1);
    }

    /// Counts past the index width are rejected with the failing count.
    #[test]
    fn next_dense_index_rejects_out_of_width_counts(
        excess in (u16::MAX as usize + 1)..(u32::MAX as usize),
    ) {
        let mut counter = excess;
        let result: Result<u16, usize> =
            oxgraph_layout_util::build::next_dense_index(&mut counter, |value| value);
        prop_assert_eq!(result, Err(excess));
        prop_assert_eq!(counter, excess);
    }
}

/// A counter at `usize::MAX` cannot advance: the allocation fails with
/// `usize::MAX` even though the value fits the index width.
#[test]
fn next_dense_index_rejects_counter_overflow() {
    let mut counter = usize::MAX;
    let result: Result<u64, usize> =
        oxgraph_layout_util::build::next_dense_index(&mut counter, |value| value);
    assert_eq!(result, Err(usize::MAX));
}
