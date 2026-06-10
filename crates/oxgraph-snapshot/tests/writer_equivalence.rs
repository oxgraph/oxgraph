//! Writer-internal equivalence laws: for any logical section list, streaming a
//! payload through chunked [`SectionSink::write`] calls must produce bytes
//! identical to the one-shot [`SnapshotWriter::section_bytes`] convenience
//! (pinning the incremental-CRC continuation law), and an under-filled
//! reservation must still open and resolve every section.

use oxgraph_layout_util::crc32c_append;
use oxgraph_snapshot::{Snapshot, SnapshotWriter};
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
    /// Chunked streaming sink output is byte-identical to one-shot writes.
    #[test]
    fn chunked_sink_matches_one_shot_bytes(
        sections in sections_strategy(),
        chunk in 1usize..16,
    ) {
        let mut one_shot =
            SnapshotWriter::new(sections.len(), crc32c_append).expect("reservation fits");
        for (index, section) in sections.iter().enumerate() {
            one_shot
                .section_bytes(
                    u32::try_from(index).expect("section index fits u32"),
                    section.version,
                    section.alignment_log2,
                    &section.payload,
                )
                .expect("writer accepts section");
        }
        let whole = one_shot.finish().expect("writer encodes");

        let mut chunked =
            SnapshotWriter::new(sections.len(), crc32c_append).expect("reservation fits");
        for (index, section) in sections.iter().enumerate() {
            let mut sink = chunked
                .begin_section(
                    u32::try_from(index).expect("section index fits u32"),
                    section.version,
                    section.alignment_log2,
                )
                .expect("writer accepts section");
            for piece in section.payload.chunks(chunk) {
                sink.write(piece);
            }
            sink.end().expect("entry fits");
        }
        let streamed = chunked.finish().expect("writer encodes");

        prop_assert_eq!(streamed, whole);
    }

    /// An under-filled reservation still opens, and every section resolves
    /// with its exact payload (unused table slots are never-referenced slack).
    #[test]
    fn underfilled_reservation_opens(sections in sections_strategy(), slack in 1usize..8) {
        let mut writer =
            SnapshotWriter::new(sections.len() + slack, crc32c_append).expect("reservation fits");
        for (index, section) in sections.iter().enumerate() {
            let mut sink = writer
                .begin_section(u32::try_from(index).expect("section index fits u32"), section.version, section.alignment_log2)
                .expect("writer accepts section");
            sink.write(&section.payload);
            sink.end().expect("entry fits");
        }
        let bytes = writer.finish().expect("writer encodes");

        let snapshot = Snapshot::open(&bytes).expect("slack layout validates");
        prop_assert_eq!(snapshot.sections().len(), sections.len());
        for (index, section) in sections.iter().enumerate() {
            let found = snapshot
                .section(u32::try_from(index).expect("section index fits u32"))
                .expect("written section resolves");
            prop_assert_eq!(found.bytes(), section.payload.as_slice());
            prop_assert_eq!(found.version(), section.version);
        }
    }
}
