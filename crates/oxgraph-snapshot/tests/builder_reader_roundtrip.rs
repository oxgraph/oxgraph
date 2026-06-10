//! Builder/reader roundtrip property test.

use oxgraph_layout_util::crc32c_append;
use oxgraph_snapshot::{MAX_ALIGNMENT_LOG2, Snapshot, SnapshotBuilder};
use proptest::prelude::*;

prop_compose! {
    fn arb_section()(
        kind in any::<u32>(),
        version in any::<u32>(),
        alignment_log2 in 0u8..=MAX_ALIGNMENT_LOG2,
        payload in proptest::collection::vec(any::<u8>(), 0..256),
    ) -> (u32, u32, u8, Vec<u8>) {
        (kind, version, alignment_log2, payload)
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: None,
        cases: 64,
        ..ProptestConfig::default()
    })]

    /// A builder followed by a reader must yield identical section payloads.
    /// Sections are added in ascending kind order (the v2 mandate).
    #[test]
    fn builder_reader_roundtrip(
        mut sections in proptest::collection::vec(arb_section(), 0..16)
            .prop_filter(
                "kinds must be unique",
                |entries| {
                    let mut seen = std::collections::BTreeSet::new();
                    entries.iter().all(|(kind, _, _, _)| seen.insert(*kind))
                },
            )
    ) {
        sections.sort_by_key(|(kind, _, _, _)| *kind);
        let mut builder = SnapshotBuilder::new(crc32c_append);
        for (kind, version, alignment_log2, payload) in &sections {
            match builder.add_section(*kind, *version, *alignment_log2, payload.clone()) {
                Ok(_) => {}
                Err(error) => panic!("builder rejected validated section: {error:?}"),
            }
        }
        let bytes = match builder.finish() {
            Ok(value) => value,
            Err(error) => panic!("builder finish failed on validated input: {error:?}"),
        };
        let snapshot = match Snapshot::open_checked(&bytes, crc32c_append) {
            Ok(value) => value,
            Err(error) => panic!("snapshot did not open: {error:?}"),
        };
        if let Err(error) = snapshot.verify_all(crc32c_append) {
            panic!("freshly built snapshot failed payload verification: {error:?}");
        }

        prop_assert_eq!(snapshot.section_count(), sections.len());
        for (kind, version, _alignment_log2, payload) in &sections {
            let Some(section) = snapshot.section(*kind) else {
                panic!("section {kind} missing after roundtrip");
            };
            prop_assert_eq!(section.kind(), *kind);
            prop_assert_eq!(section.version(), *version);
            prop_assert_eq!(section.bytes(), payload.as_slice());
            prop_assert_eq!(section.expected_crc32c(), crc32c_append(0, payload));
            prop_assert!(section.verify(crc32c_append).is_ok());
        }
    }
}
