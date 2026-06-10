//! v2 checksum coverage: any payload byte flip is caught by the verify
//! paths naming the failing kind, any table byte flip is caught by
//! `open_checked`, and `patch_section_crc` restores the invariants after a
//! deliberate post-encode payload patch.

use oxgraph_layout_util::crc32c_append;
use oxgraph_snapshot::{
    HEADER_SIZE, SECTION_ENTRY_SIZE, SectionViewError, Snapshot, SnapshotError, SnapshotWriter,
    patch_section_crc,
};
use proptest::{prelude::*, test_runner::TestCaseError};

/// Kinds of the three baseline sections, in ascending order.
const KINDS: [u32; 3] = [3, 7, 9];

/// Builds a three-section snapshot with distinct non-empty payloads.
fn baseline() -> Vec<u8> {
    let mut writer = match SnapshotWriter::new(KINDS.len(), crc32c_append) {
        Ok(value) => value,
        Err(error) => panic!("writer: {error:?}"),
    };
    for (index, kind) in KINDS.iter().enumerate() {
        let payload = vec![u8::try_from(index).unwrap_or(0); 8 + index];
        if let Err(error) = writer.section_bytes(*kind, 0, 0, &payload) {
            panic!("section_bytes({kind}): {error:?}");
        }
    }
    match writer.finish() {
        Ok(bytes) => bytes,
        Err(error) => panic!("writer finish: {error:?}"),
    }
}

/// Returns the payload byte range of section `kind` within `bytes`.
fn payload_range(bytes: &[u8], kind: u32) -> core::ops::Range<usize> {
    let snapshot = match Snapshot::open(bytes) {
        Ok(value) => value,
        Err(error) => panic!("baseline must open: {error:?}"),
    };
    let Some(section) = snapshot.section(kind) else {
        panic!("section {kind} missing");
    };
    let start = section.bytes().as_ptr().addr() - bytes.as_ptr().addr();
    start..start + section.bytes().len()
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    /// Flipping any payload byte of any section fails `verify_all` and
    /// `Section::verify` with an error naming the corrupted kind, while the
    /// structural open still succeeds (it never scans payloads).
    #[test]
    fn payload_byte_flip_fails_verify_naming_kind(
        which in 0usize..KINDS.len(),
        byte in any::<proptest::sample::Index>(),
        bit in 0u32..8,
    ) {
        let mut bytes = baseline();
        let kind = KINDS[which];
        let range = payload_range(&bytes, kind);
        let offset = range.start + byte.index(range.len());
        bytes[offset] ^= 1u8 << bit;

        // Structural open and the table checksum are payload-blind.
        let snapshot = match Snapshot::open_checked(&bytes, crc32c_append) {
            Ok(value) => value,
            Err(error) => panic!("payload corruption must not fail open_checked: {error:?}"),
        };

        match snapshot.verify_all(crc32c_append) {
            Err(SnapshotError::SectionChecksumMismatch { kind: failing, .. }) => {
                prop_assert_eq!(failing, kind);
            }
            other => return Err(TestCaseError::fail(format!(
                "expected SectionChecksumMismatch for kind {kind}, got {other:?}"
            ))),
        }

        let Some(section) = snapshot.section(kind) else {
            return Err(TestCaseError::fail("corrupted section must still resolve"));
        };
        match section.verify(crc32c_append) {
            Err(SectionViewError::ChecksumMismatch { kind: failing, expected, actual }) => {
                prop_assert_eq!(failing, kind);
                prop_assert_ne!(expected, actual);
            }
            other => return Err(TestCaseError::fail(format!(
                "expected ChecksumMismatch for kind {kind}, got {other:?}"
            ))),
        }
    }

    /// Flipping any section-table byte is never silently accepted by
    /// `open_checked`: either structural validation rejects the table or the
    /// table checksum mismatches.
    #[test]
    fn table_byte_flip_fails_open_checked(
        byte in any::<proptest::sample::Index>(),
        bit in 0u32..8,
    ) {
        let mut bytes = baseline();
        let table_len = KINDS.len() * SECTION_ENTRY_SIZE;
        let offset = HEADER_SIZE + byte.index(table_len);
        bytes[offset] ^= 1u8 << bit;
        prop_assert!(Snapshot::open_checked(&bytes, crc32c_append).is_err());
    }
}

/// After mutating a payload post-encode, `patch_section_crc` restores both
/// the section's entry checksum and the header table checksum.
#[test]
fn patch_section_crc_restores_invariants() -> Result<(), SnapshotError> {
    let mut bytes = baseline();
    let range = payload_range(&bytes, KINDS[2]);
    bytes[range.start] ^= 0xFF;

    // Corrupted: payload verification fails, naming the patched kind.
    {
        let snapshot = Snapshot::open_checked(&bytes, crc32c_append)?;
        match snapshot.verify_all(crc32c_append) {
            Err(SnapshotError::SectionChecksumMismatch { kind, .. }) => {
                assert_eq!(kind, KINDS[2]);
            }
            other => panic!("expected SectionChecksumMismatch, got {other:?}"),
        }
    }

    patch_section_crc(&mut bytes, KINDS[2], crc32c_append)?;
    let snapshot = Snapshot::open_checked(&bytes, crc32c_append)?;
    snapshot.verify_all(crc32c_append)?;
    Ok(())
}

/// `patch_section_crc` reports a missing kind with a typed error.
#[test]
fn patch_section_crc_rejects_missing_kind() {
    let mut bytes = baseline();
    match patch_section_crc(&mut bytes, 0xDEAD, crc32c_append) {
        Err(SnapshotError::SectionMissing { kind: 0xDEAD }) => {}
        other => panic!("expected SectionMissing, got {other:?}"),
    }
}
