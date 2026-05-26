//! Kani proofs for inbound CSC snapshot views.

#![cfg(kani)]

use oxgraph_snapshot::Snapshot;

use crate::CscSnapshotGraph;

/// Opening inbound sections from a dual-topology snapshot succeeds on a tiny graph.
#[kani::proof]
fn csc_open_total_on_tiny_graph() {
    static BYTES: &[u8] = include_bytes!("proofs/tiny_dual.oxgtopo");
    let Ok(snapshot) = Snapshot::open(BYTES) else {
        panic!("fixture snapshot must open");
    };
    let Ok(graph) = CscSnapshotGraph::from_snapshot(&snapshot) else {
        panic!("fixture graph must open");
    };
    assert_eq!(graph.node_count(), 2);
    assert_eq!(graph.relation_count(), 1);
}
