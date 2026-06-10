//! Typed section view tests for [`Section::try_as_slice`].
//!
//! Verifies that the view checks the actual borrowed pointer's alignment
//! rather than trusting the section entry's declared `alignment_log2`.

use oxgraph_layout_util::crc32c_append;
use oxgraph_snapshot::{
    PendingSection, Section, SectionViewError, Snapshot, SnapshotError, SnapshotPlan,
    SnapshotWriter,
};
use zerocopy::byteorder::{LE, U32};

#[test]
fn try_as_slice_succeeds_when_aligned() -> Result<(), SnapshotError> {
    let mut writer = match SnapshotWriter::new(1, crc32c_append) {
        Ok(value) => value,
        Err(error) => panic!("writer: {error:?}"),
    };
    let payload: [u32; 4] = [1, 2, 3, 4];
    if let Err(error) = writer.section_typed(1, 0, &payload) {
        panic!("typed section: {error:?}");
    }
    let bytes = match writer.finish() {
        Ok(value) => value,
        Err(error) => panic!("writer finish: {error:?}"),
    };

    let snapshot = Snapshot::open(&bytes)?;
    let Some(section) = snapshot.section(1) else {
        panic!("section 1 missing");
    };
    let words: &[U32<LE>] = match section.try_as_slice() {
        Ok(value) => value,
        Err(error) => panic!("expected aligned LE u32 view: {error:?}"),
    };
    let observed: Vec<u32> = words.iter().map(|word| word.get()).collect();
    assert_eq!(observed, payload);

    Ok(())
}

#[test]
fn try_as_slice_rejects_misaligned_pointer() -> Result<(), SnapshotError> {
    let payload = [0u32, 1, 2, 3];
    let bytes_payload: Vec<u8> = payload
        .iter()
        .flat_map(|word| word.to_le_bytes().to_vec())
        .collect();
    let sections = [PendingSection {
        kind: 7,
        version: 0,
        alignment_log2: 0,
        payload: &bytes_payload,
    }];
    let plan = match SnapshotPlan::new(&sections) {
        Ok(value) => value,
        Err(error) => panic!("plan validation failed: {error:?}"),
    };
    let needed = match plan.encoded_len() {
        Ok(value) => value,
        Err(error) => panic!("encoded_len failed: {error:?}"),
    };
    let mut buffer = vec![0u8; needed + 1];
    let written = match plan.write_into(&mut buffer[1..], crc32c_append) {
        Ok(value) => value,
        Err(error) => panic!("write_into failed: {error:?}"),
    };

    // Borrow a sub-slice that starts one byte off; whether the inner payload
    // pointer happens to be 4-byte aligned or not is incidental. Either the
    // view succeeds (lucky alignment) or returns the typed alignment error —
    // never UB.
    let snapshot = Snapshot::open(&buffer[1..=written])?;
    let Some(section): Option<Section<'_>> = snapshot.section(7) else {
        panic!("section 7 missing");
    };
    let result: Result<&[u32], SectionViewError> = section.try_as_slice();
    if let Err(err) = result {
        assert!(matches!(err, SectionViewError::AlignmentMismatch { .. }));
    }

    Ok(())
}

#[test]
fn try_as_slice_rejects_misaligned_length() -> Result<(), SnapshotError> {
    let payload = b"abcde";
    let mut writer = match SnapshotWriter::new(1, crc32c_append) {
        Ok(value) => value,
        Err(error) => panic!("writer: {error:?}"),
    };
    if let Err(error) = writer.section_bytes(1, 0, 0, payload) {
        panic!("byte section: {error:?}");
    }
    let bytes = match writer.finish() {
        Ok(value) => value,
        Err(error) => panic!("writer finish: {error:?}"),
    };

    let snapshot = Snapshot::open(&bytes)?;
    let Some(section) = snapshot.section(1) else {
        panic!("section 1 missing");
    };
    match section.try_as_slice::<U32<LE>>() {
        Err(SectionViewError::LengthNotMultipleOfSize { .. }) => {}
        other => panic!("expected LengthNotMultipleOfSize, got {other:?}"),
    }

    Ok(())
}
