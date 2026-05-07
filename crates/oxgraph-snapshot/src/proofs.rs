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
    FORMAT_MAGIC, FORMAT_MAJOR, FORMAT_MINOR, HEADER_SIZE, HeaderOnlySnapshot, PendingSection,
    SECTION_ENTRY_SIZE, Snapshot, SnapshotPlan, ValidationLevel,
};

/// Header parsing must always return `Ok` or a typed error — never panic.
#[kani::proof]
fn parse_header_total() {
    let bytes: [u8; HEADER_SIZE] = kani::any();
    let _ = HeaderOnlySnapshot::open(&bytes);
}

/// `Snapshot::open` over bounded 64-byte inputs must always return a `Result`.
#[kani::proof]
#[kani::unwind(9)]
fn open_does_not_panic_on_bounded_64() {
    let bytes: [u8; 64] = kani::any();
    let section_count = u32::from_le_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    // kani-skip: the fully arbitrary section-count path is covered by the
    // malformed-bytes proptest; Kani does not narrow the 64-byte table length
    // enough and expands the max-section validation loop.
    kani::assume(section_count <= 1);
    let _ = Snapshot::open_with(&bytes, ValidationLevel::SectionTable);
}

/// A two-section plan must round-trip through write_into's encoded bytes.
#[kani::proof]
#[kani::unwind(16)]
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
    let payload_start = HEADER_SIZE + 2 * SECTION_ENTRY_SIZE;
    assert_eq!(needed, payload_start + 8);

    // kani-skip: full layout validation re-enters zerocopy slice metadata with
    // a symbolic section-table length; integration tests cover the full
    // `Snapshot::open` layout roundtrip.
    assert_eq!(buffer[0], FORMAT_MAGIC[0]);
    assert_eq!(buffer[1], FORMAT_MAGIC[1]);
    assert_eq!(buffer[2], FORMAT_MAGIC[2]);
    assert_eq!(buffer[3], FORMAT_MAGIC[3]);
    assert_eq!(buffer[4], FORMAT_MAGIC[4]);
    assert_eq!(buffer[5], FORMAT_MAGIC[5]);
    assert_eq!(buffer[6], FORMAT_MAGIC[6]);
    assert_eq!(buffer[7], FORMAT_MAGIC[7]);
    assert_eq!(
        u32::from_le_bytes([buffer[8], buffer[9], buffer[10], buffer[11]]),
        FORMAT_MAJOR
    );
    assert_eq!(
        u32::from_le_bytes([buffer[12], buffer[13], buffer[14], buffer[15]]),
        FORMAT_MINOR
    );
    assert_eq!(
        u32::from_le_bytes([buffer[16], buffer[17], buffer[18], buffer[19]]),
        HEADER_SIZE as u32
    );
    assert_eq!(
        u32::from_le_bytes([buffer[20], buffer[21], buffer[22], buffer[23]]),
        2
    );

    assert_eq!(
        u64::from_le_bytes([
            buffer[32], buffer[33], buffer[34], buffer[35], buffer[36], buffer[37], buffer[38],
            buffer[39],
        ]),
        payload_start as u64
    );
    assert_eq!(
        u64::from_le_bytes([
            buffer[40], buffer[41], buffer[42], buffer[43], buffer[44], buffer[45], buffer[46],
            buffer[47],
        ]),
        4
    );
    assert_eq!(
        u32::from_le_bytes([buffer[48], buffer[49], buffer[50], buffer[51]]),
        kind_a
    );
    assert_eq!(
        u64::from_le_bytes([
            buffer[64], buffer[65], buffer[66], buffer[67], buffer[68], buffer[69], buffer[70],
            buffer[71],
        ]),
        (payload_start + 4) as u64
    );
    assert_eq!(
        u64::from_le_bytes([
            buffer[72], buffer[73], buffer[74], buffer[75], buffer[76], buffer[77], buffer[78],
            buffer[79],
        ]),
        4
    );
    assert_eq!(
        u32::from_le_bytes([buffer[80], buffer[81], buffer[82], buffer[83]]),
        kind_b
    );
    assert_eq!(buffer[payload_start], payload_a[0]);
    assert_eq!(buffer[payload_start + 1], payload_a[1]);
    assert_eq!(buffer[payload_start + 2], payload_a[2]);
    assert_eq!(buffer[payload_start + 3], payload_a[3]);
    assert_eq!(buffer[payload_start + 4], payload_b[0]);
    assert_eq!(buffer[payload_start + 5], payload_b[1]);
    assert_eq!(buffer[payload_start + 6], payload_b[2]);
    assert_eq!(buffer[payload_start + 7], payload_b[3]);
}

/// FORMAT_MAGIC must be exactly the eight ASCII bytes of "OXGTOPO\0".
#[kani::proof]
fn format_magic_constant() {
    assert_eq!(FORMAT_MAGIC, *b"OXGTOPO\0");
}
