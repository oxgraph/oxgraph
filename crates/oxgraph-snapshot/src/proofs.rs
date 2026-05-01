//! Kani proof harnesses for the snapshot container.
//!
//! Each `#[kani::proof]` exercises a bounded scenario the container must
//! never violate: header parsing must be total, section table validation
//! must be sound for small N, range arithmetic must not overflow, and
//! plan-write/reader-open must round-trip for fixed-shape inputs.
//!
//! These proofs run under `cargo kani` (heavy gate, not in `just ci`).

#![cfg(kani)]

use crate::container::{
    FORMAT_MAGIC, FORMAT_MAJOR, FORMAT_MINOR, HEADER_SIZE, PendingSection, SECTION_ENTRY_SIZE,
    Snapshot, SnapshotPlan,
};

/// Header parsing must always return `Ok` or a typed error — never panic.
#[kani::proof]
fn parse_header_total() {
    let bytes: [u8; HEADER_SIZE] = kani::any();
    let _ = Snapshot::open(&bytes);
}

/// `Snapshot::open` over arbitrary 64-byte inputs must always return a `Result`.
#[kani::proof]
#[kani::unwind(2)]
fn open_does_not_panic_on_arbitrary_64() {
    let bytes: [u8; 64] = kani::any();
    let _ = Snapshot::open(&bytes);
}

/// A two-section plan must round-trip through write_into and Snapshot::open.
#[kani::proof]
#[kani::unwind(8)]
fn plan_write_open_roundtrip_n2() {
    let kind_a: u32 = kani::any();
    let kind_b: u32 = kani::any();
    kani::assume(kind_a != kind_b);

    let payload_a: [u8; 4] = kani::any();
    let payload_b: [u8; 4] = kani::any();

    let sections = [
        PendingSection {
            kind: kind_a,
            version: 0,
            alignment_log2: 0,
            payload: &payload_a,
        },
        PendingSection {
            kind: kind_b,
            version: 0,
            alignment_log2: 0,
            payload: &payload_b,
        },
    ];

    let plan = SnapshotPlan::new(&sections).unwrap();
    let needed = plan.encoded_len().unwrap();
    let mut buffer = [0u8; HEADER_SIZE + 2 * SECTION_ENTRY_SIZE + 8];
    assert!(needed <= buffer.len());
    plan.write_into(&mut buffer[..needed]).unwrap();

    let snapshot = Snapshot::open(&buffer[..needed]).unwrap();
    assert_eq!(snapshot.format_major(), FORMAT_MAJOR);
    assert_eq!(snapshot.format_minor(), FORMAT_MINOR);
    assert_eq!(snapshot.section_count(), 2);
    assert_eq!(
        snapshot.section(kind_a).map(|s| s.bytes()),
        Some(&payload_a[..])
    );
    assert_eq!(
        snapshot.section(kind_b).map(|s| s.bytes()),
        Some(&payload_b[..])
    );
}

/// FORMAT_MAGIC must be exactly the eight ASCII bytes of "OXGTOPO\0".
#[kani::proof]
fn format_magic_constant() {
    assert_eq!(FORMAT_MAGIC, *b"OXGTOPO\0");
}
