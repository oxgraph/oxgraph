//! No-`alloc` smoke test: build, write, and re-open a snapshot in a stack
//! buffer using [`SnapshotPlan`].

use oxgraph_snapshot::{
    PendingSection, PlanError, Snapshot, SnapshotError, SnapshotPlan, ValidationLevel,
};

#[test]
fn round_trips_two_sections_in_stack_buffer() -> Result<(), SnapshotError> {
    let payload_a = [0x11u8, 0x22, 0x33, 0x44];
    let payload_b = [0xAAu8, 0xBB];
    let sections = [
        PendingSection {
            kind: 7,
            version: 1,
            alignment_log2: 2,
            payload: &payload_a,
        },
        PendingSection {
            kind: 9,
            version: 0,
            alignment_log2: 0,
            payload: &payload_b,
        },
    ];

    let plan = match SnapshotPlan::new(&sections) {
        Ok(value) => value,
        Err(error) => panic!("plan validation failed: {error:?}"),
    };
    let needed = match plan.encoded_len() {
        Ok(value) => value,
        Err(error) => panic!("encoded_len failed: {error:?}"),
    };
    assert!(needed <= 256);

    let mut buffer = [0u8; 256];
    let written = match plan.write_into(&mut buffer) {
        Ok(value) => value,
        Err(error) => panic!("write_into failed: {error:?}"),
    };
    assert_eq!(written, needed);

    let snapshot = Snapshot::open(&buffer[..needed])?;
    assert_eq!(snapshot.section_count(), 2);
    assert_eq!(snapshot.section(7).map(|s| s.bytes()), Some(&payload_a[..]));
    assert_eq!(snapshot.section(9).map(|s| s.bytes()), Some(&payload_b[..]));
    assert_eq!(snapshot.section(7).map(|s| s.version()), Some(1));

    Ok(())
}

#[test]
fn rejects_buffer_too_small() {
    let payload = [0u8; 8];
    let sections = [PendingSection {
        kind: 1,
        version: 0,
        alignment_log2: 0,
        payload: &payload,
    }];
    let plan = match SnapshotPlan::new(&sections) {
        Ok(value) => value,
        Err(error) => panic!("plan validation failed: {error:?}"),
    };
    let mut tiny = [0u8; 16];
    match plan.write_into(&mut tiny) {
        Err(PlanError::BufferTooSmall { .. }) => {}
        other => panic!("expected BufferTooSmall, got {other:?}"),
    }
}

#[test]
fn rejects_duplicate_kind_at_plan_construction() {
    let payload = [0u8; 1];
    let sections = [
        PendingSection {
            kind: 5,
            version: 0,
            alignment_log2: 0,
            payload: &payload,
        },
        PendingSection {
            kind: 5,
            version: 7,
            alignment_log2: 0,
            payload: &payload,
        },
    ];
    match SnapshotPlan::new(&sections) {
        Err(PlanError::DuplicateKind { kind: 5 }) => {}
        other => panic!("expected DuplicateKind {{ kind: 5 }}, got {other:?}"),
    }
}

#[test]
fn header_only_validation_skips_section_table() -> Result<(), SnapshotError> {
    let payload = [0u8; 4];
    let sections = [PendingSection {
        kind: 1,
        version: 0,
        alignment_log2: 0,
        payload: &payload,
    }];
    let plan = match SnapshotPlan::new(&sections) {
        Ok(value) => value,
        Err(error) => panic!("plan validation failed: {error:?}"),
    };
    let mut buffer = [0u8; 128];
    let written = match plan.write_into(&mut buffer) {
        Ok(value) => value,
        Err(error) => panic!("write_into failed: {error:?}"),
    };
    let snapshot = Snapshot::open_with(&buffer[..written], ValidationLevel::Header)?;
    assert_eq!(snapshot.section_count(), 0);
    Ok(())
}
