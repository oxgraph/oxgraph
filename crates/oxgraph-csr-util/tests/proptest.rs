//! Property tests for offset-integrity primitives.

use oxgraph_csr_util::{
    OffsetIntegrityIssue, check_offset_section, check_offsets_monotonic, check_value_range,
};
use proptest::{prelude::*, test_runner::TestCaseError};

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

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
