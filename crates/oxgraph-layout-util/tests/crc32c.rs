//! Tests pinning `crc32c_append` to the CRC-32C (Castagnoli) algorithm.
//!
//! The software implementation must be byte-for-byte interchangeable with the
//! hardware-accelerated `crc32c` crate: snapshot writers may fold payload
//! checksums with either, and readers verify with whichever they hold.

use oxgraph_layout_util::crc32c_append;
use proptest::prelude::*;

/// The published CRC-32C check value for the ASCII string `123456789` is
/// `0xE306_9283`; matching it pins the polynomial, bit reflection, and final
/// XOR.
#[test]
fn known_check_vector() {
    assert_eq!(crc32c_append(0, b"123456789"), 0xE306_9283);
}

/// The empty input with seed zero yields zero, so an absent or empty section
/// stores a zero checksum.
#[test]
fn empty_input_is_zero() {
    assert_eq!(crc32c_append(0, b""), 0);
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    /// The software fold equals the `crc32c` crate's hardware-backed fold for
    /// any seed and input, so the two implementations are interchangeable on
    /// both the write and verify paths.
    #[test]
    fn matches_crc32c_crate(
        seed in any::<u32>(),
        bytes in proptest::collection::vec(any::<u8>(), 0..2048),
    ) {
        prop_assert_eq!(crc32c_append(seed, &bytes), crc32c::crc32c_append(seed, &bytes));
    }

    /// Continuation law: folding a split input in two steps equals folding the
    /// whole input once. Snapshot writers rely on this to checksum streamed
    /// payloads incrementally.
    #[test]
    fn continuation_matches_single_shot(
        bytes in proptest::collection::vec(any::<u8>(), 0..512),
        split in any::<proptest::sample::Index>(),
    ) {
        let mid = split.index(bytes.len() + 1);
        let (head, tail) = bytes.split_at(mid);
        prop_assert_eq!(
            crc32c_append(crc32c_append(0, head), tail),
            crc32c_append(0, &bytes)
        );
    }
}
