//! Kani proofs for inbound CSC snapshot views.

#![cfg(kani)]

use oxgraph_snapshot::Snapshot;

use crate::CscSnapshotGraph;

/// Inbound offsets section kind used by the proof fixture (Postgres band).
const FIXTURE_OFFSETS_KIND: u32 = 0x0201;
/// Inbound targets section kind used by the proof fixture (Postgres band).
const FIXTURE_TARGETS_KIND: u32 = 0x0202;
/// CSR section version used by the proof fixture.
const FIXTURE_VERSION: u32 = 1;

/// Opening inbound sections from a dual-topology snapshot succeeds on a tiny graph.
#[kani::proof]
fn csc_open_total_on_tiny_graph() {
    static BYTES: &[u8] = include_bytes!("proofs/tiny_dual.oxgtopo");
    let Ok(snapshot) = Snapshot::open(BYTES) else {
        panic!("fixture snapshot must open");
    };
    let Ok(graph) = CscSnapshotGraph::<u32, u32>::from_snapshot_with_kinds(
        &snapshot,
        FIXTURE_OFFSETS_KIND,
        FIXTURE_TARGETS_KIND,
        FIXTURE_VERSION,
    ) else {
        panic!("fixture graph must open");
    };
    assert_eq!(graph.node_count(), 2);
    assert_eq!(graph.relation_count(), 1);
}
