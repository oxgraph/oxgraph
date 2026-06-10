//! Packs two sections into a snapshot, opens it, and reads them back.
//!
//! The first section holds little-endian `u32` words; the second holds raw
//! bytes. Demonstrates the topology-agnostic surface — neither section
//! has graph or hypergraph semantics.
//!
//! Run with: `cargo run -p oxgraph-snapshot --example pack_two_sections --features alloc`

use oxgraph_layout_util::crc32c_append;
use oxgraph_snapshot::{Snapshot, SnapshotBuilder, SnapshotError};
use zerocopy::byteorder::{LE, U32};

fn main() -> Result<(), SnapshotError> {
    let words: [u32; 5] = [10, 20, 30, 40, 50];
    let bytes_payload = b"oxgraph-snapshot v2.0".to_vec();

    let mut builder = SnapshotBuilder::new(crc32c_append);
    if let Err(error) = builder.add_section_typed(0x0100, 0, &words) {
        panic!("typed section: {error:?}");
    }
    if let Err(error) = builder.add_section(0x0101, 0, 0, bytes_payload) {
        panic!("byte section: {error:?}");
    }
    let snapshot_bytes = match builder.finish() {
        Ok(value) => value,
        Err(error) => panic!("builder finish: {error:?}"),
    };
    println!("encoded snapshot: {} bytes", snapshot_bytes.len());

    let snapshot = Snapshot::open(&snapshot_bytes)?;
    println!(
        "format v{}.{}, {} section(s)",
        snapshot.format_major(),
        snapshot.format_minor(),
        snapshot.section_count()
    );

    for section in snapshot.sections() {
        println!(
            "  kind={} version={} bytes={}",
            section.kind(),
            section.version(),
            section.bytes().len()
        );
    }

    let Some(words_section) = snapshot.section(0x0100) else {
        panic!("words section missing");
    };
    let observed_words: &[U32<LE>] = match words_section.try_as_slice() {
        Ok(slice) => slice,
        Err(error) => panic!("words section view failed: {error:?}"),
    };
    let observed: Vec<u32> = observed_words.iter().map(|word| word.get()).collect();
    println!("words section: {observed:?}");

    let Some(bytes_section) = snapshot.section(0x0101) else {
        panic!("bytes section missing");
    };
    println!(
        "bytes section: {}",
        core::str::from_utf8(bytes_section.bytes()).unwrap_or("<invalid utf8>")
    );

    Ok(())
}
