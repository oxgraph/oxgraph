//! Equivalence law between the two encoders: for any logical section list,
//! [`SnapshotWriter`] (write-through, exact table reservation) must produce
//! bytes identical to [`SnapshotBuilder`] (own-then-copy), and an
//! under-filled reservation must still open and resolve every section.

use oxgraph_snapshot::{Snapshot, SnapshotBuilder, SnapshotWriter};
use proptest::prelude::*;

/// One arbitrary logical section: a unique kind is assigned by index.
#[derive(Clone, Debug)]
struct ArbSection {
    /// Section version word.
    version: u32,
    /// Declared payload alignment (log2), within the format cap.
    alignment_log2: u8,
    /// Raw payload bytes.
    payload: Vec<u8>,
}

/// Strategy producing 1..8 arbitrary sections.
fn sections_strategy() -> impl Strategy<Value = Vec<ArbSection>> {
    proptest::collection::vec(
        (
            any::<u32>(),
            0u8..=4,
            proptest::collection::vec(any::<u8>(), 0..64),
        )
            .prop_map(|(version, alignment_log2, payload)| ArbSection {
                version,
                alignment_log2,
                payload,
            }),
        1..8,
    )
}

proptest! {
    /// Exact-reservation writer output is byte-identical to the builder's.
    #[test]
    fn writer_matches_builder_bytes(sections in sections_strategy()) {
        let mut builder = SnapshotBuilder::new();
        for (index, section) in sections.iter().enumerate() {
            builder
                .add_section(
                    index as u32,
                    section.version,
                    section.alignment_log2,
                    section.payload.clone(),
                )
                .expect("builder accepts section");
        }
        let built = builder.finish().expect("builder encodes");

        let mut writer = SnapshotWriter::new(sections.len()).expect("reservation fits");
        for (index, section) in sections.iter().enumerate() {
            let mut sink = writer
                .begin_section(index as u32, section.version, section.alignment_log2)
                .expect("writer accepts section");
            sink.write(&section.payload);
            sink.end().expect("entry fits");
        }
        let written = writer.finish().expect("writer encodes");

        prop_assert_eq!(written, built);
    }

    /// An under-filled reservation still opens, and every section resolves
    /// with its exact payload (unused table slots are never-referenced slack).
    #[test]
    fn underfilled_reservation_opens(sections in sections_strategy(), slack in 1usize..8) {
        let mut writer =
            SnapshotWriter::new(sections.len() + slack).expect("reservation fits");
        for (index, section) in sections.iter().enumerate() {
            let mut sink = writer
                .begin_section(index as u32, section.version, section.alignment_log2)
                .expect("writer accepts section");
            sink.write(&section.payload);
            sink.end().expect("entry fits");
        }
        let bytes = writer.finish().expect("writer encodes");

        let snapshot = Snapshot::open(&bytes).expect("slack layout validates");
        prop_assert_eq!(snapshot.sections().len(), sections.len());
        for (index, section) in sections.iter().enumerate() {
            let found = snapshot
                .section(index as u32)
                .expect("written section resolves");
            prop_assert_eq!(found.bytes(), section.payload.as_slice());
            prop_assert_eq!(found.version(), section.version);
        }
    }
}
