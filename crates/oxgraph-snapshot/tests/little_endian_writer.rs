//! Tests for explicit little-endian section writer helpers.

use oxgraph_layout_util::crc32c_append;
use oxgraph_snapshot::{Snapshot, SnapshotWriter};
use zerocopy::byteorder::{LE, U16, U32, U64};

#[test]
fn little_endian_helper_writes_portable_word_bytes() {
    let payload = [
        U16::<LE>::new(0x0102),
        U16::<LE>::new(0x0304),
        U16::<LE>::new(0x0506),
    ];
    let mut writer = match SnapshotWriter::new(1, crc32c_append) {
        Ok(writer) => writer,
        Err(error) => panic!("writer construction failed: {error:?}"),
    };
    if let Err(error) = writer.section_little_endian(0xCAFE, 7, &payload) {
        panic!("little-endian section rejected: {error:?}");
    }
    let bytes = match writer.finish() {
        Ok(bytes) => bytes,
        Err(error) => panic!("snapshot finish failed: {error:?}"),
    };
    let snapshot = match Snapshot::open(&bytes) {
        Ok(snapshot) => snapshot,
        Err(error) => panic!("snapshot open failed: {error:?}"),
    };
    let section = snapshot
        .section(0xCAFE)
        .unwrap_or_else(|| panic!("little-endian section missing"));
    assert_eq!(section.version(), 7);
    assert_eq!(section.bytes(), &[0x02, 0x01, 0x04, 0x03, 0x06, 0x05]);
}

#[test]
fn little_endian_helper_supports_multiple_word_widths() {
    let words32 = [U32::<LE>::new(0x0102_0304)];
    let words64 = [U64::<LE>::new(0x0102_0304_0506_0708)];
    let mut writer = match SnapshotWriter::new(2, crc32c_append) {
        Ok(writer) => writer,
        Err(error) => panic!("writer construction failed: {error:?}"),
    };
    if let Err(error) = writer.section_little_endian(1, 0, &words32) {
        panic!("u32 little-endian section rejected: {error:?}");
    }
    if let Err(error) = writer.section_little_endian(2, 0, &words64) {
        panic!("u64 little-endian section rejected: {error:?}");
    }
    let bytes = match writer.finish() {
        Ok(bytes) => bytes,
        Err(error) => panic!("snapshot finish failed: {error:?}"),
    };
    let snapshot = match Snapshot::open(&bytes) {
        Ok(snapshot) => snapshot,
        Err(error) => panic!("snapshot open failed: {error:?}"),
    };
    assert_eq!(
        snapshot.section(1).map(|section| section.bytes()),
        Some(&[0x04, 0x03, 0x02, 0x01][..])
    );
    assert_eq!(
        snapshot.section(2).map(|section| section.bytes()),
        Some(&[0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01][..])
    );
}
