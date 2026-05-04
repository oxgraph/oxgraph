//! Wire-layout regression test.
//!
//! These assertions defend the byte-level format constants against silent
//! drift. The internal `RawHeader` / `RawSectionEntry` types are crate-private
//! (the only way to consume them is through the validated [`Snapshot`] API),
//! so this test exercises the layout from the public surface: it builds
//! snapshots with known section counts and payload sizes and asserts the
//! encoded byte length matches the formula
//! `HEADER_SIZE + section_count * SECTION_ENTRY_SIZE + sum(payload_len)`
//! (modulo per-section padding for declared alignment).
//!
//! The zerocopy derives on the internal raw structs guarantee the per-field
//! layout is correct *today*; this test ensures the next person who
//! refactors the wire types catches any size or magic regression that would
//! break readers in the wild.

use oxgraph_snapshot::{
    FORMAT_MAGIC, FORMAT_MAJOR, FORMAT_MINOR, HEADER_SIZE, SECTION_ENTRY_SIZE, SnapshotBuilder,
};

/// An empty snapshot is exactly `HEADER_SIZE` bytes.
#[test]
fn empty_snapshot_is_exactly_header_size() {
    let builder = SnapshotBuilder::new();
    let bytes = match builder.finish() {
        Ok(value) => value,
        Err(error) => panic!("empty builder finish: {error:?}"),
    };
    assert_eq!(bytes.len(), HEADER_SIZE);
}

/// The first eight bytes of any valid snapshot equal `FORMAT_MAGIC`.
#[test]
fn snapshot_magic_is_at_offset_zero() {
    let builder = SnapshotBuilder::new();
    let bytes = match builder.finish() {
        Ok(value) => value,
        Err(error) => panic!("empty builder finish: {error:?}"),
    };
    assert!(bytes.len() >= FORMAT_MAGIC.len());
    assert_eq!(&bytes[..FORMAT_MAGIC.len()], &FORMAT_MAGIC[..]);
}

/// `FORMAT_MAGIC` is exactly the eight ASCII bytes of "OXGTOPO\0".
#[test]
fn format_magic_is_oxgtopo_nul() {
    assert_eq!(FORMAT_MAGIC, *b"OXGTOPO\0");
}

/// `HEADER_SIZE` and `SECTION_ENTRY_SIZE` are 32 bytes each; reduction below
/// that would require a major-version bump and a coordinated reader update.
#[test]
fn header_and_section_entry_sizes_are_pinned() {
    assert_eq!(HEADER_SIZE, 32);
    assert_eq!(SECTION_ENTRY_SIZE, 32);
}

/// `FORMAT_MAJOR` and `FORMAT_MINOR` are the v1 baseline; raising either
/// without updating consumers would silently break readers in the wild.
#[test]
fn format_versions_are_pinned() {
    assert_eq!(FORMAT_MAJOR, 1);
    assert_eq!(FORMAT_MINOR, 0);
}

/// Encoded length follows the wire-layout formula:
/// `HEADER_SIZE + section_count * SECTION_ENTRY_SIZE + sum(payload_len)`
/// when no section requires padding. With `alignment_log2 = 0`, the
/// payloads are tightly packed.
#[test]
fn one_section_is_header_plus_entry_plus_payload() {
    let mut builder = SnapshotBuilder::new();
    let payload = vec![0xABu8; 17];
    if let Err(error) = builder.add_section(0x1234, 0, 0, payload.clone()) {
        panic!("add_section: {error:?}");
    }
    let bytes = match builder.finish() {
        Ok(value) => value,
        Err(error) => panic!("builder finish: {error:?}"),
    };
    assert_eq!(
        bytes.len(),
        HEADER_SIZE + SECTION_ENTRY_SIZE + payload.len()
    );
}

/// With three unpadded sections, the formula extends linearly.
#[test]
fn three_sections_is_header_plus_three_entries_plus_payloads() {
    let mut builder = SnapshotBuilder::new();
    let payloads: [Vec<u8>; 3] = [vec![1u8; 4], vec![2u8; 8], vec![3u8; 12]];
    for (kind, payload) in payloads.iter().enumerate() {
        let kind = match u32::try_from(kind) {
            Ok(value) => value,
            Err(error) => panic!("kind index out of u32 range: {error:?}"),
        };
        if let Err(error) = builder.add_section(kind, 0, 0, payload.clone()) {
            panic!("add_section({kind}): {error:?}");
        }
    }
    let bytes = match builder.finish() {
        Ok(value) => value,
        Err(error) => panic!("builder finish: {error:?}"),
    };
    let payload_total: usize = payloads.iter().map(Vec::len).sum();
    assert_eq!(
        bytes.len(),
        HEADER_SIZE + 3 * SECTION_ENTRY_SIZE + payload_total
    );
}
