//! Builder determinism property test: equal logical input → byte-equal output.

use oxgraph_layout_util::crc32c_append;
use oxgraph_snapshot::{MAX_ALIGNMENT_LOG2, SnapshotBuilder};
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

fn build_once(sections: &[(u32, u32, u8, Vec<u8>)]) -> Vec<u8> {
    let mut builder = SnapshotBuilder::new(crc32c_append);
    for (kind, version, alignment_log2, payload) in sections {
        match builder.add_section(*kind, *version, *alignment_log2, payload.clone()) {
            Ok(_) => {}
            Err(error) => panic!("builder rejected validated section: {error:?}"),
        }
    }
    match builder.finish() {
        Ok(bytes) => bytes,
        Err(error) => panic!("builder finish failed on validated input: {error:?}"),
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: None,
        cases: 64,
        ..ProptestConfig::default()
    })]

    #[test]
    fn builder_is_deterministic(
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
        let first = build_once(&sections);
        let second = build_once(&sections);
        prop_assert_eq!(first, second);
    }
}
