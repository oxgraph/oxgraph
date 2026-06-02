//! CRC-32C integrity primitive for the OXGDB delta-log and superblock.
//!
//! The on-disk format binds every delta-log record and the superblock to a
//! CRC-32C (Castagnoli polynomial `0x1EDC6F41`) checksum so recovery can reject
//! torn or corrupted bytes deterministically. This module is the single source
//! of the checksum so the writer and reader agree on the exact algorithm. It
//! wraps the hardware-accelerated [`crc32c`] crate, which selects an
//! SSE4.2/ARMv8 implementation at runtime and falls back to a software table.
//!
//! # Performance
//!
//! `perf: unspecified`; the wrapper itself is `O(1)` and delegates an `O(n)`
//! scan to [`crc32c::crc32c`].

/// Computes the CRC-32C (Castagnoli) checksum over `bytes`.
///
/// The result is the value the OXGDB format stores little-endian in a record's
/// `crc32c` field. It is byte-order independent (the checksum is over the byte
/// sequence, not over multi-byte words), so it is stable across architectures.
///
/// # Performance
///
/// This function is `O(n)` in `bytes.len()`.
//
// kani-skip: delegates to the external `crc32c` crate whose runtime CPU-feature
// dispatch and unbounded scan are outside the model checker's reach; the known
// check vector, determinism, and single-bit-flip detection are proved by the
// unit tests below instead.
pub(crate) fn checksum(bytes: &[u8]) -> u32 {
    crc32c::crc32c(bytes)
}

/// Continues a CRC-32C (Castagnoli) checksum over `bytes`, seeded with the result
/// of a prior [`checksum`]/`checksum_append` call. Chaining
/// `checksum_append(checksum(a), b)` equals `checksum(&[a, b].concat())` because
/// [`checksum`] is itself `crc32c_append(0, ..)`; this lets a caller checksum two
/// non-contiguous slices (a record's header prefix and its trailing
/// ops+blob) without allocating a joined buffer on the data path.
///
/// # Performance
///
/// This function is `O(n)` in `bytes.len()`.
//
// kani-skip: delegates to the external `crc32c` crate whose runtime CPU-feature
// dispatch and unbounded scan are outside the model checker's reach, exactly as
// [`checksum`]; the seed-continuation algebra it relies on is the crate's
// documented contract and is exercised by the unit test below.
pub(crate) fn checksum_append(seed: u32, bytes: &[u8]) -> u32 {
    crc32c::crc32c_append(seed, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The published CRC-32C check value for the ASCII string `123456789` is
    /// `0xE306_9283`; matching it pins the polynomial, bit reflection, and
    /// final XOR of the wrapped implementation.
    #[test]
    fn known_check_vector() {
        assert_eq!(checksum(b"123456789"), 0xE306_9283);
    }

    /// The empty input has the canonical CRC-32C seed result of zero.
    #[test]
    fn empty_input_is_zero() {
        assert_eq!(checksum(b""), 0);
    }

    /// Hashing identical bytes twice yields identical checksums; recovery relies
    /// on recomputation matching the stored value byte for byte.
    #[test]
    fn deterministic() {
        let buffer = b"oxgraph delta-log integrity check vector";
        assert_eq!(checksum(buffer), checksum(buffer));
    }

    /// Continuing a checksum with `checksum_append` over a split of a buffer
    /// equals the single-shot checksum of the whole buffer, so the delta-log can
    /// CRC a record's two non-contiguous regions without joining them.
    #[test]
    fn append_matches_single_shot() {
        let whole = b"oxgraph delta-log record header prefix and trailing ops plus blob";
        for split in 0..=whole.len() {
            let (head, tail) = whole.split_at(split);
            assert_eq!(checksum_append(checksum(head), tail), checksum(whole));
        }
    }

    /// Flipping any single bit of a multi-byte buffer changes the checksum, so a
    /// single-bit corruption inside a record is always detected.
    #[test]
    fn single_bit_flip_detected() {
        let original: [u8; 16] = [
            0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD,
            0xEE, 0xFF,
        ];
        let baseline = checksum(&original);
        for byte_index in 0..original.len() {
            for bit in 0..8u32 {
                let mut mutated = original;
                mutated[byte_index] ^= 1u8 << bit;
                assert_ne!(
                    checksum(&mutated),
                    baseline,
                    "bit {bit} of byte {byte_index} went undetected",
                );
            }
        }
    }
}
