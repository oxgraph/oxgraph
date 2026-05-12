//! Property tests for builder helper primitives.

use oxgraph_build_util::{
    BuildIndex, IdOutOfBounds, OffsetOverflow, build_offset_index, id_to_slot, index_from_usize,
    slot_or_max,
};
use proptest::{prelude::*, test_runner::TestCaseError};

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

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
        if let Some(value) = <u64 as BuildIndex>::to_usize(id) {
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

    /// `BuildIndex::from_usize` ∘ `BuildIndex::to_usize` is identity for `u32`.
    #[test]
    fn build_index_roundtrip_u32(value in 0_usize..(u32::MAX as usize + 1)) {
        let Some(index) = <u32 as BuildIndex>::from_usize(value) else {
            return Err(TestCaseError::fail("value should fit u32"));
        };
        let Some(back) = <u32 as BuildIndex>::to_usize(index) else {
            return Err(TestCaseError::fail("u32 always fits usize"));
        };
        prop_assert_eq!(back, value);
    }
}
