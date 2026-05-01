//! Snapshot opener tests covering header, section table, and layout failures.

use oxgraph_snapshot::{
    FORMAT_MAGIC, FORMAT_MAJOR, FORMAT_MINOR, HEADER_SIZE, MAX_ALIGNMENT_LOG2, MAX_SECTION_COUNT,
    PlanError, SECTION_ENTRY_SIZE, Snapshot, SnapshotBuilder, SnapshotError,
};

/// Convenience to add a section while panicking on builder errors.
fn add(builder: &mut SnapshotBuilder, kind: u32, version: u32, alignment_log2: u8, payload: &[u8]) {
    if let Err(error) = builder.add_section(kind, version, alignment_log2, payload.to_vec()) {
        let formatted: PlanError = error;
        panic!("add_section({kind}): {formatted:?}");
    }
}

/// Builds a known-good snapshot with two distinct sections.
fn baseline_snapshot() -> Vec<u8> {
    let mut builder = SnapshotBuilder::new();
    add(&mut builder, 1, 0, 2, b"abcd");
    add(&mut builder, 2, 0, 0, b"xyz");
    builder.finish()
}

/// Overwrites a little-endian `u32` at `offset` in `bytes`.
fn set_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

/// Asserts that opening `bytes` produces exactly `expected`.
fn assert_open_error(bytes: &[u8], expected: &SnapshotError) {
    match Snapshot::open(bytes) {
        Ok(_) => panic!("expected {expected:?}, got Ok"),
        Err(actual) => assert_eq!(&actual, expected),
    }
}

/// Returns the `u32` representation of `HEADER_SIZE` for this build.
fn header_size_u32() -> u32 {
    match u32::try_from(HEADER_SIZE) {
        Ok(value) => value,
        Err(error) => panic!("HEADER_SIZE does not fit u32: {error:?}"),
    }
}

#[test]
fn opens_baseline_snapshot() -> Result<(), SnapshotError> {
    let bytes = baseline_snapshot();
    let snapshot = Snapshot::open(&bytes)?;

    assert_eq!(snapshot.format_major(), FORMAT_MAJOR);
    assert_eq!(snapshot.format_minor(), FORMAT_MINOR);
    assert_eq!(snapshot.section_count(), 2);
    assert_eq!(
        snapshot.section(1).map(|s| s.bytes()),
        Some(b"abcd".as_ref())
    );
    assert_eq!(
        snapshot.section(2).map(|s| s.bytes()),
        Some(b"xyz".as_ref())
    );
    assert!(snapshot.section(3).is_none());

    Ok(())
}

#[test]
fn rejects_truncated_header() {
    assert_open_error(
        &[0u8; 8],
        &SnapshotError::TruncatedHeader {
            needed: HEADER_SIZE,
            actual: 8,
        },
    );
}

#[test]
fn rejects_bad_magic() {
    let mut bytes = baseline_snapshot();
    bytes[0] = 0;
    let mut expected = FORMAT_MAGIC;
    expected[0] = 0;
    assert_open_error(&bytes, &SnapshotError::BadMagic { actual: expected });
}

#[test]
fn rejects_wrong_major() {
    let mut bytes = baseline_snapshot();
    set_u32(&mut bytes, 8, FORMAT_MAJOR + 1);
    assert_open_error(
        &bytes,
        &SnapshotError::FormatMajorMismatch {
            actual: FORMAT_MAJOR + 1,
            supported: FORMAT_MAJOR,
        },
    );
}

#[test]
fn rejects_minor_too_new() {
    let mut bytes = baseline_snapshot();
    set_u32(&mut bytes, 12, FORMAT_MINOR + 1);
    assert_open_error(
        &bytes,
        &SnapshotError::FormatMinorTooNew {
            actual: FORMAT_MINOR + 1,
            max_supported: FORMAT_MINOR,
        },
    );
}

#[test]
fn rejects_wrong_header_size() {
    let mut bytes = baseline_snapshot();
    set_u32(&mut bytes, 16, 64);
    assert_open_error(
        &bytes,
        &SnapshotError::HeaderSizeMismatch {
            actual: 64,
            expected: header_size_u32(),
        },
    );
}

#[test]
fn rejects_non_zero_header_reserved() {
    let mut bytes = baseline_snapshot();
    bytes[24] = 1;
    assert_open_error(&bytes, &SnapshotError::NonZeroHeaderReserved);
}

#[test]
fn rejects_section_count_too_large() {
    let mut bytes = baseline_snapshot();
    set_u32(&mut bytes, 20, MAX_SECTION_COUNT + 1);
    assert_open_error(
        &bytes,
        &SnapshotError::SectionCountTooLarge {
            count: MAX_SECTION_COUNT + 1,
            max: MAX_SECTION_COUNT,
        },
    );
}

#[test]
fn rejects_truncated_section_table() {
    let mut bytes = baseline_snapshot();
    bytes.truncate(HEADER_SIZE + SECTION_ENTRY_SIZE);
    match Snapshot::open(&bytes) {
        Err(SnapshotError::TruncatedSectionTable { .. }) => {}
        other => panic!("expected TruncatedSectionTable, got {other:?}"),
    }
}

#[test]
fn rejects_non_zero_entry_checksum() {
    let mut bytes = baseline_snapshot();
    let entry_offset = HEADER_SIZE;
    bytes[entry_offset + 24] = 1;
    assert_open_error(&bytes, &SnapshotError::NonZeroEntryChecksum { kind: 1 });
}

#[test]
fn rejects_unsupported_flags() {
    let mut bytes = baseline_snapshot();
    let entry_offset = HEADER_SIZE;
    bytes[entry_offset + 29] = 0b0000_0001;
    assert_open_error(
        &bytes,
        &SnapshotError::UnsupportedFlags {
            kind: 1,
            flags: 0b0000_0001,
        },
    );
}

#[test]
fn rejects_non_zero_entry_reserved() {
    let mut bytes = baseline_snapshot();
    let entry_offset = HEADER_SIZE;
    bytes[entry_offset + 30] = 1;
    assert_open_error(&bytes, &SnapshotError::NonZeroEntryReserved { kind: 1 });
}

#[test]
fn rejects_alignment_log2_too_large() {
    let mut bytes = baseline_snapshot();
    let entry_offset = HEADER_SIZE;
    bytes[entry_offset + 28] = MAX_ALIGNMENT_LOG2 + 1;
    assert_open_error(
        &bytes,
        &SnapshotError::AlignmentLog2TooLarge {
            kind: 1,
            alignment_log2: MAX_ALIGNMENT_LOG2 + 1,
        },
    );
}

#[test]
fn rejects_section_out_of_bounds() {
    let mut bytes = baseline_snapshot();
    let entry_offset = HEADER_SIZE;
    let huge = (bytes.len() as u64).wrapping_add(1024);
    bytes[entry_offset + 8..entry_offset + 16].copy_from_slice(&huge.to_le_bytes());
    match Snapshot::open(&bytes) {
        Err(SnapshotError::SectionOutOfBounds { kind: 1, .. }) => {}
        other => panic!("expected SectionOutOfBounds, got {other:?}"),
    }
}

#[test]
fn rejects_unsorted_section_table() {
    let mut bytes = baseline_snapshot();
    let first = HEADER_SIZE;
    let second = HEADER_SIZE + SECTION_ENTRY_SIZE;
    let mut tmp = [0u8; SECTION_ENTRY_SIZE];
    tmp.copy_from_slice(&bytes[first..first + SECTION_ENTRY_SIZE]);
    bytes.copy_within(second..second + SECTION_ENTRY_SIZE, first);
    bytes[second..second + SECTION_ENTRY_SIZE].copy_from_slice(&tmp);
    match Snapshot::open(&bytes) {
        Err(SnapshotError::UnsortedSectionTable { .. }) => {}
        other => panic!("expected UnsortedSectionTable, got {other:?}"),
    }
}

#[test]
fn rejects_duplicate_kind() {
    let mut bytes = baseline_snapshot();
    let second_entry = HEADER_SIZE + SECTION_ENTRY_SIZE;
    set_u32(&mut bytes, second_entry + 16, 1);
    assert_open_error(&bytes, &SnapshotError::DuplicateKind { kind: 1 });
}
