//! Writer determinism property test: equal logical input → byte-equal output.

use oxgraph_layout_util::crc32c_append;
use oxgraph_snapshot::{MAX_ALIGNMENT_LOG2, SnapshotWriter};
use proptest::prelude::*;

prop_compose! {
    fn arb_section()(
        kind in any::<u32>(),
        version in any::<u32>(),
        alignment_log2 in 0u8..=MAX_ALIGNMENT_LOG2,
        payload in proptest::collection::vec(any::<u8>(), 0..128),
    ) -> (u32, u32, u8, Vec<u8>) {
        (kind, version, alignment_log2, payload)
    }
}

fn write_once(sections: &[(u32, u32, u8, Vec<u8>)]) -> Vec<u8> {
    let mut writer = match SnapshotWriter::new(sections.len(), crc32c_append) {
        Ok(writer) => writer,
        Err(error) => panic!("writer rejected validated reservation: {error:?}"),
    };
    for (kind, version, alignment_log2, payload) in sections {
        if let Err(error) = writer.section_bytes(*kind, *version, *alignment_log2, payload) {
            panic!("writer rejected validated section: {error:?}");
        }
    }
    match writer.finish() {
        Ok(bytes) => bytes,
        Err(error) => panic!("writer finish failed on validated input: {error:?}"),
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: None,
        cases: 64,
        ..ProptestConfig::default()
    })]

    #[test]
    fn writer_is_deterministic(
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
        let first = write_once(&sections);
        let second = write_once(&sections);
        prop_assert_eq!(first, second);
    }
}
