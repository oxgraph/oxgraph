//! Panic-freedom property test: arbitrary byte input must never panic.
//!
//! This is the §22.1 keystone invariant — malformed snapshots must not be
//! able to cause undefined behaviour or panics through the safe API.

use oxgraph_snapshot::Snapshot;
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: None,
        cases: 256,
        ..ProptestConfig::default()
    })]

    /// `Snapshot::open` over arbitrary bytes returns a `Result` and never panics.
    #[test]
    fn open_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..4096)) {
        let _ignored = Snapshot::open(&bytes);
    }
}
